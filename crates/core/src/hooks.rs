use crate::history;
use crate::ipc::{UnixListener, UnixStream};
use crate::process;
use crate::reminders;
use crate::agent::{
    AgentEvent, AgentStatus, Message, Stage, Subject, Usage, CONNECTOR_ENV, ISSUE_ENV, SLACK_CHANNEL_ENV, SLACK_THREAD_ENV, SOCK_ENV, TAB_ENV,
};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const SUBCOMMAND: &str = "hook";
pub const STAGE_SUBCOMMAND: &str = "stage";
pub const SLACK_SUBCOMMAND: &str = "slack";
pub const LINEAR_SUBCOMMAND: &str = "linear";
pub const STATUSLINE_SUBCOMMAND: &str = "statusline";

/// What the harness answers to each socket line.
type Reply = Result<String, String>;

/// How long a prompt waits for the project memory the harness recalls for it. Past this the prompt
/// goes on without it.
const RECALL_WAIT: Duration = Duration::from_millis(300);

/// Claude Code settings layered onto the harnessed session via `--settings`, so the user's
/// own ~/.claude/settings.json stays untouched.
pub fn write_settings(path: &Path, exe: &Path) -> std::io::Result<()> {
    let mut hooks = serde_json::Map::new();
    for (_, arg, hook_event) in AgentStatus::ALL {
        hooks.insert(hook_event.into(), serde_json::json!([{ "hooks": [hook_command(exe, arg)] }]));
    }
    std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({
        "hooks": hooks,
        "statusLine": { "type": "command", "command": statusline_command(exe) },
        "permissions": { "allow": reminders::permissions() },
    }))?)
}

/// Shell form, which every Claude Code version runs.
#[cfg(unix)]
fn hook_command(exe: &Path, arg: &str) -> serde_json::Value {
    serde_json::json!({ "type": "command", "command": format!("'{}' {SUBCOMMAND} {arg}", exe.display()) })
}

/// Exec form, with no shell: Windows runs a shell-form hook in Git Bash, or in PowerShell when
/// Git Bash is missing, and the two quote a path differently.
#[cfg(windows)]
fn hook_command(exe: &Path, arg: &str) -> serde_json::Value {
    serde_json::json!({ "type": "command", "command": exe, "args": [SUBCOMMAND, arg] })
}

/// A statusline has only shell form. Git Bash, which runs it on Windows, reads a backslash as an escape.
fn statusline_command(exe: &Path) -> String {
    let exe = exe.display().to_string();
    #[cfg(windows)]
    let exe = exe.replace('\\', "/");
    format!("'{exe}' {STATUSLINE_SUBCOMMAND}")
}

/// Runs inside the hook process Claude Code spawns: read the hook payload from stdin and
/// forward it to the tab that owns this session, and print what Claude Code should add to a
/// prompt. Never fails loudly, so it can't disturb the agent.
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

    if let Some(output) = deliver(&sock, event) {
        println!("{output}");
    }
}

/// Sends the hook's event. A prompt waits up to `RECALL_WAIT` for the memories the harness
/// recalls for it, returned as the hook's JSON output.
fn deliver(sock: &str, event: AgentEvent) -> Option<String> {
    if event.status != AgentStatus::Working || event.prompt.is_none() {
        tell(sock, &Message::Agent(event));
        return None;
    }
    let mut stream = UnixStream::connect(sock).ok()?;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    writeln!(stream, "{}", serde_json::to_string(&Message::Agent(event)).unwrap()).ok()?;
    let _ = stream.set_read_timeout(Some(RECALL_WAIT));
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).ok()?;
    let context = serde_json::from_str::<Reply>(&reply).ok()?.ok().filter(|c| !c.is_empty())?;
    Some(serde_json::json!({ "hookSpecificOutput": { "hookEventName": "UserPromptSubmit", "additionalContext": context } }).to_string())
}

/// Sends one message to the running harness without waiting for its answer.
fn tell(sock: &str, message: &Message) {
    if let Ok(mut stream) = UnixStream::connect(sock) {
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let _ = writeln!(stream, "{}", serde_json::to_string(message).unwrap());
    }
}

/// Runs as the session's statusline: forwards the subscription usage Claude Code reports, then
/// prints the user's own statusline, which the `--settings` entry would otherwise replace.
pub fn statusline() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if let (Some(usage), Ok(sock)) = (usage_from_status(&input), std::env::var(SOCK_ENV)) {
        tell(&sock, &Message::Usage { usage, account: std::env::var(crate::accounts::ACCOUNT_ENV).ok() });
    }
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map_or_else(|| crate::machine::home().join(".claude"), PathBuf::from);
    if let Some(command) = user_statusline(&config_dir) {
        let _ = std::io::stdout().write_all(&run_statusline(&command, &input));
    }
}

/// The usage in the JSON Claude Code pipes to its statusline. It has none until the session's
/// first API response, and none at all without a Pro or Max subscription.
fn usage_from_status(input: &str) -> Option<Usage> {
    #[derive(serde::Deserialize)]
    struct Status {
        rate_limits: Option<Usage>,
    }
    serde_json::from_str::<Status>(input).ok()?.rate_limits.filter(|u| u.five_hour.is_some() || u.seven_day.is_some())
}

/// The statusline command set in the user's own Claude Code settings.
fn user_statusline(config_dir: &Path) -> Option<String> {
    let settings: serde_json::Value = serde_json::from_slice(&std::fs::read(config_dir.join("settings.json")).ok()?).ok()?;
    settings["statusLine"]["command"].as_str().map(str::to_owned)
}

/// What `command` prints when given the statusline JSON, run the way Claude Code runs it.
fn run_statusline(command: &str, input: &str) -> Vec<u8> {
    let Some(shell) = statusline_shell() else { return vec![] };
    let Ok(mut child) = process::command(shell)
        .args(["-c", command])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
    else {
        return vec![];
    };
    // A command that never reads its input closes the pipe early, and still gets to print.
    let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    child.wait_with_output().map(|out| out.stdout).unwrap_or_default()
}

#[cfg(unix)]
fn statusline_shell() -> Option<PathBuf> {
    Some("/bin/sh".into())
}

/// The Git Bash that runs pinned's statusline puts its own bash first on PATH. Searched here
/// because Windows would try System32 before PATH, and WSL's bash lives there.
#[cfg(windows)]
fn statusline_shell() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join("bash.exe")).find(|p| p.is_file())
}

const LIMIT_ERROR: &[u8] = br#""apiError":"usage_limit_reached""#;
const API_ERROR: &[u8] = br#""isApiErrorMessage":true"#;

/// When the usage limit that stopped the session lifts, from the last error Claude Code wrote to
/// its transcript, unless that error was another, such as a monthly spend limit with no reset.
/// Only lines carrying an error are parsed.
fn limit_reset_from_transcript(transcript: impl BufRead) -> Option<u64> {
    let has = |line: &[u8], mark: &[u8]| line.windows(mark.len()).any(|w| w == mark);
    let last = transcript
        .split(b'\n')
        .map_while(Result::ok)
        .filter(|line| has(line, LIMIT_ERROR) || has(line, API_ERROR))
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(&line).ok())
        .last()?;
    last["apiErrorParams"]["rate_limit_info"]["resetsAt"].as_u64().filter(|_| last["apiError"] == "usage_limit_reached")
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

/// Runs when a Slack connector's session calls `pinned linear create [--assignee <who>] <title>
/// <description>`, to file a Linear issue for its thread, or `pinned linear take <identifier>`, to
/// work on an existing issue no other session works on. Returns what the harness answered.
pub fn linear(args: &[String]) -> Reply {
    let usage = || format!("usage: pinned {LINEAR_SUBCOMMAND} create [--assignee <who>] <title> <description> | take <identifier>");
    let context = || -> Result<_, String> { Ok((env(CONNECTOR_ENV)?, env(SLACK_CHANNEL_ENV)?, env(SLACK_THREAD_ENV)?)) };
    match args {
        [verb, rest @ ..] if verb == "create" => {
            let (assignee, rest) = match rest {
                [flag, who, rest @ ..] if flag == "--assignee" => (who.clone(), rest),
                rest => (String::new(), rest),
            };
            let [title, description @ ..] = rest else { return Err(usage()) };
            if title.trim().is_empty() || description.is_empty() {
                return Err(usage());
            }
            let (connector, channel, thread) = context()?;
            ask(Message::LinearCreate { connector, channel, thread, title: title.clone(), description: description.join(" "), assignee })
        }
        [verb, issue] if verb == "take" && !issue.trim().is_empty() => {
            let (connector, channel, thread) = context()?;
            ask(Message::LinearTake { connector, channel, thread, issue: issue.clone() })
        }
        _ => Err(usage()),
    }
}

fn env(k: &str) -> Result<String, String> {
    std::env::var(k).map_err(|_| format!("{k} is not set: run this inside a session a pinned workflow started"))
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
    use crate::agent::LimitWindow;

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
        let spent = format!("{transcript}\n{}", r#"{"type":"assistant","isApiErrorMessage":true,"error":"rate_limit","message":{"content":[{"type":"text","text":"You've hit your monthly spend limit"}]}}"#);
        assert_eq!(limit_reset_from_transcript(spent.as_bytes()), None, "a later error without a reset kept the old one");
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
            Message::LinearCreate { connector, channel, thread, title, description, assignee } => {
                Ok(format!("{connector} filed {title:?} / {description:?} for {channel}:{thread} to {assignee:?}"))
            }
            Message::LinearTake { connector, channel, thread, issue } => Ok(format!("{connector} took {issue} for {channel}:{thread}")),
            other => Err(format!("unexpected {other:?}")),
        })
        .unwrap();
        std::env::set_var(SOCK_ENV, &sock);
        std::env::set_var(CONNECTOR_ENV, "c1");
        assert_eq!(stage(Some("coding")), Err("PINNED_ISSUE is not set: run this inside a session a pinned workflow started".to_owned()));
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
            Err("PINNED_SLACK_CHANNEL is not set: run this inside a session a pinned workflow started".to_owned())
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
            Ok("c1 filed \"Export 500s\" / \"The export button fails\" for C0123ABCD:1792400050.000200 to \"\"".to_owned())
        );
        assert_eq!(
            linear(&args(&["create", "--assignee", "Ana Souza", "Export 500s", "The export button", "fails"])),
            Ok("c1 filed \"Export 500s\" / \"The export button fails\" for C0123ABCD:1792400050.000200 to \"Ana Souza\"".to_owned())
        );
        assert_eq!(linear(&args(&["take", "ENG-123"])), Ok("c1 took ENG-123 for C0123ABCD:1792400050.000200".to_owned()));
        let usage = Err("usage: pinned linear create [--assignee <who>] <title> <description> | take <identifier>".to_owned());
        assert_eq!(linear(&args(&["create", "Export 500s"])), usage);
        assert_eq!(linear(&args(&["take"])), usage);
        assert_eq!(linear(&args(&["create", "--assignee", "Ana", "Export 500s"])), usage);
        assert_eq!(linear(&args(&["file", "a", "b"])), usage);
        let _ = std::fs::remove_file(sock);
    }

    #[test]
    fn a_prompt_gets_the_memory_the_harness_recalls_unless_it_takes_too_long() {
        let sock = std::env::temp_dir().join(format!("pinned-recall-{}.sock", std::process::id()));
        listen(sock.clone(), |message| match message {
            Message::Agent(AgentEvent { prompt: Some(p), .. }) if p == "slow" => {
                std::thread::sleep(Duration::from_secs(2));
                Ok("[1] Late: too late".into())
            }
            Message::Agent(AgentEvent { prompt: Some(p), .. }) if p == "nothing known" => Ok(String::new()),
            Message::Agent(AgentEvent { prompt: Some(p), .. }) => Ok(format!("[1] Deploys: the hub deploys from tags ({p})")),
            _ => Ok("not for a hook that does not wait".into()),
        })
        .unwrap();
        let sock = sock.display().to_string();
        let event = |status, prompt: Option<&str>| AgentEvent { status, cwd: None, message: None, tab: Some("tab-1".into()), session: None, title: None, resume_at: None, prompt: prompt.map(Into::into), snapshot: None };
        assert_eq!(
            deliver(&sock, event(AgentStatus::Working, Some("how do we deploy?"))),
            Some(r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"[1] Deploys: the hub deploys from tags (how do we deploy?)"}}"#.into())
        );
        assert_eq!(deliver(&sock, event(AgentStatus::Working, Some("nothing known"))), None);
        assert_eq!(deliver(&sock, event(AgentStatus::Done, Some("how do we deploy?"))), None);
        let start = std::time::Instant::now();
        assert_eq!(deliver(&sock, event(AgentStatus::Working, Some("slow"))), None);
        assert!(start.elapsed() < Duration::from_millis(800), "the prompt waited {:?}", start.elapsed());
        assert_eq!(deliver("/nonexistent/pinned.sock", event(AgentStatus::Working, Some("how do we deploy?"))), None);
        let _ = std::fs::remove_file(sock);
    }

    #[test]
    fn statusline_usage_is_read_from_rate_limits() {
        let both = r#"{"session_id":"s1","model":{"display_name":"Opus"},"rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1792400000},"seven_day":{"used_percentage":41.2,"resets_at":1792900000}}}"#;
        assert_eq!(
            usage_from_status(both),
            Some(Usage {
                five_hour: Some(LimitWindow { used_percentage: 23.5, resets_at: 1792400000 }),
                seven_day: Some(LimitWindow { used_percentage: 41.2, resets_at: 1792900000 }),
            })
        );
        let weekly_only = r#"{"rate_limits":{"seven_day":{"used_percentage":90,"resets_at":1792900000}}}"#;
        assert_eq!(
            usage_from_status(weekly_only),
            Some(Usage { five_hour: None, seven_day: Some(LimitWindow { used_percentage: 90.0, resets_at: 1792900000 }) })
        );
        assert_eq!(usage_from_status(r#"{"session_id":"s1","cwd":"/repo"}"#), None);
        assert_eq!(usage_from_status(r#"{"rate_limits":{}}"#), None);
        assert_eq!(usage_from_status(r#"{"rate_limits":{"five_hour":{"used_percentage":"lots"}}}"#), None);
        assert_eq!(usage_from_status("{broken"), None);
        assert_eq!(usage_from_status(""), None);
    }

    #[test]
    fn the_users_own_statusline_runs_on_the_same_input() {
        let dir = std::env::temp_dir().join(format!("pinned-statusline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(user_statusline(&dir), None);
        std::fs::write(dir.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        assert_eq!(user_statusline(&dir), None);
        std::fs::write(dir.join("settings.json"), r#"{"statusLine":{"type":"command","command":"tr a-z A-Z"}}"#).unwrap();
        let command = user_statusline(&dir).unwrap();
        assert_eq!(command, "tr a-z A-Z");
        assert_eq!(run_statusline(&command, r#"{"cwd":"/repo"}"#), br#"{"CWD":"/REPO"}"#);
        assert_eq!(run_statusline("echo done", &"x".repeat(1 << 20)), b"done\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn settings_run_the_hooks_and_statusline_through_the_shell() {
        let file = std::env::temp_dir().join(format!("pinned-settings-{}.json", std::process::id()));
        write_settings(&file, Path::new("/Applications/pinned.app/Contents/MacOS/pinned")).unwrap();
        let settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(
            settings["hooks"]["StopFailure"],
            serde_json::json!([{ "hooks": [{ "type": "command", "command": "'/Applications/pinned.app/Contents/MacOS/pinned' hook failed" }] }])
        );
        assert_eq!(
            settings["statusLine"],
            serde_json::json!({ "type": "command", "command": "'/Applications/pinned.app/Contents/MacOS/pinned' statusline" })
        );
        let _ = std::fs::remove_file(file);
    }

    #[cfg(windows)]
    #[test]
    fn settings_run_the_hooks_without_a_shell_and_the_statusline_in_git_bash() {
        let file = std::env::temp_dir().join(format!("pinned-settings-{}.json", std::process::id()));
        write_settings(&file, Path::new(r"C:\Users\Ana Souza\AppData\Local\pinned\pinned.exe")).unwrap();
        let settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(
            settings["hooks"]["StopFailure"],
            serde_json::json!([{ "hooks": [{
                "type": "command",
                "command": r"C:\Users\Ana Souza\AppData\Local\pinned\pinned.exe",
                "args": ["hook", "failed"],
            }] }])
        );
        assert_eq!(
            settings["statusLine"],
            serde_json::json!({ "type": "command", "command": "'C:/Users/Ana Souza/AppData/Local/pinned/pinned.exe' statusline" })
        );
        let _ = std::fs::remove_file(file);
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
