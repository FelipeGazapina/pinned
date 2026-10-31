use super::*;

pub(crate) struct Driver {
    pub(crate) args: fn(&SpawnSpec) -> Vec<String>,
    pub(crate) parse: fn(&str) -> Vec<StreamEvent>,
    pub(crate) resume: fn(&str) -> Vec<String>,
}

pub fn driver_args(provider: Provider, spec: &SpawnSpec) -> Vec<String> {
    (driver(provider).args)(spec)
}

pub fn resume_args(provider: Provider, session: &str) -> Vec<String> {
    (driver(provider).resume)(session)
}

pub(crate) fn driver(provider: Provider) -> &'static Driver {
    match provider {
        Provider::Claude => &CLAUDE,
        Provider::Codex => &CODEX,
        Provider::Cursor => &CURSOR,
    }
}

const CLAUDE: Driver = Driver {
    args: claude_args,
    parse: parse_claude,
    resume: resume_flag,
};
const CODEX: Driver = Driver {
    args: codex_args,
    parse: parse_codex,
    resume: codex_resume,
};
const CURSOR: Driver = Driver {
    args: cursor_args,
    parse: parse_cursor,
    resume: resume_flag,
};

/// A task starting with a dash would read as an option, such as a bypass flag, so it leads with a newline.
fn prompt_arg(task: &str) -> String {
    if task.starts_with('-') { format!("\n{task}") } else { task.to_owned() }
}

fn claude_args(spec: &SpawnSpec) -> Vec<String> {
    let mut args = vec![
        "-p".into(),
        prompt_arg(spec.task),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
    ];
    if let Some(model) = spec.model {
        args.extend(["--model".into(), model.into()]);
    }
    if let Some(effort) = spec.effort {
        args.extend(["--effort".into(), effort.into()]);
    }
    match spec.access {
        Access::Full => args.push("--dangerously-skip-permissions".into()),
        Access::Auto => args.extend(["--permission-mode".into(), "auto".into()]),
        Access::ReadOnly => args.extend(["--permission-mode".into(), "plan".into()]),
    }
    if let Some(config) = spec.mcp_config {
        args.extend(["--mcp-config".into(), config.display().to_string()]);
    }
    args
}

fn codex_args(spec: &SpawnSpec) -> Vec<String> {
    let mut args = vec![
        "exec".into(),
        "--json".into(),
        "--skip-git-repo-check".into(),
    ];
    if let Some(model) = spec.model {
        args.extend(["-m".into(), model.into()]);
    }
    if let Some(effort) = spec.effort {
        args.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]);
    }
    match spec.access {
        Access::Full => args.push("--dangerously-bypass-approvals-and-sandbox".into()),
        Access::Auto => args.push("--full-auto".into()),
        Access::ReadOnly => args.extend(["-s".into(), "read-only".into()]),
    }
    args.extend(codex_mcp_flags(
        spec.exe,
        spec.sock,
        None,
        Some(spec.task_id),
        spec.reminders,
    ));
    args.push(prompt_arg(spec.task));
    args
}

/// `--trust` only trusts the workspace. `--force` is the approval bypass, so auto leaves it off
/// and plan uses `--mode plan` instead.
fn cursor_args(spec: &SpawnSpec) -> Vec<String> {
    let mut args = vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--trust".into(),
    ];
    match spec.access {
        Access::Full => args.push("--force".into()),
        Access::Auto => {}
        Access::ReadOnly => args.extend(["--mode".into(), "plan".into()]),
    }
    if let Some(model) = spec.model {
        args.extend(["--model".into(), model.into()]);
    }
    args.push(prompt_arg(spec.task));
    args
}

fn resume_flag(session: &str) -> Vec<String> {
    vec!["--resume".into(), session.into()]
}

fn codex_resume(session: &str) -> Vec<String> {
    vec!["resume".into(), session.into()]
}

/// Codex reads MCP servers from `-c` config, with the socket and tab written out. It does not
/// expand a `${PINNED_SOCK}` the way Claude's mcp config does.
pub fn codex_mcp_flags(
    exe: &Path,
    sock: &Path,
    tab: Option<&str>,
    task: Option<&str>,
    reminders: bool,
) -> Vec<String> {
    let mut args = vec![
        "-c".into(),
        format!(
            "mcp_servers.pinned.command={}",
            toml_str(&exe.display().to_string())
        ),
    ];
    let mcp_args = if reminders {
        r#"["mcp"]"#
    } else {
        r#"["mcp","--no-reminders"]"#
    };
    args.extend(["-c".into(), format!("mcp_servers.pinned.args={mcp_args}")]);
    args.extend([
        "-c".into(),
        format!(
            "mcp_servers.pinned.env.PINNED_SOCK={}",
            toml_str(&sock.display().to_string())
        ),
    ]);
    if let Some(tab) = tab {
        args.extend([
            "-c".into(),
            format!("mcp_servers.pinned.env.PINNED_TAB={}", toml_str(tab)),
        ]);
    }
    if let Some(task) = task {
        args.extend([
            "-c".into(),
            format!("mcp_servers.pinned.env.PINNED_TASK={}", toml_str(task)),
        ]);
    }
    args
}

fn toml_str(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{0008}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{000c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Codex runs this when a turn finishes. The JSON payload arrives as the last argument.
/// `hook done` is the same turn-end pinned already maps for Claude's Stop hook.
pub fn codex_notify_flag(exe: &Path, user: &[String]) -> Vec<String> {
    let mut parts = vec![
        toml_str(&exe.display().to_string()),
        toml_str(crate::hooks::SUBCOMMAND),
        toml_str("done"),
    ];
    if !user.is_empty() {
        parts.push(toml_str("--"));
        parts.extend(user.iter().map(|arg| toml_str(arg)));
    }
    vec!["-c".into(), format!("notify=[{}]", parts.join(", "))]
}

/// The `notify` array in a Codex `config.toml`, when it is a top-level list of strings.
pub fn codex_user_notify(config: &str) -> Vec<String> {
    let mut in_table = false;
    let mut collecting: Option<String> = None;
    for line in config.lines() {
        if let Some(buf) = collecting.as_mut() {
            buf.push('\n');
            buf.push_str(line);
            if brackets_balanced(buf) {
                return quoted_strings(buf);
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('[') && !trimmed.contains('=') {
            in_table = true;
            continue;
        }
        if in_table {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("notify") else { continue };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let rest = rest.trim();
        if !rest.starts_with('[') {
            continue;
        }
        if brackets_balanced(rest) {
            return quoted_strings(rest);
        }
        collecting = Some(rest.to_owned());
    }
    vec![]
}

fn brackets_balanced(text: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == '\\' && q == '"' {
                chars.next();
                continue;
            }
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => {}
        }
    }
    depth == 0 && quote.is_none()
}

fn quoted_strings(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '"' && c != '\'' {
            continue;
        }
        let quote = c;
        let mut value = String::new();
        while let Some(n) = chars.next() {
            if n == '\\' && quote == '"' {
                match chars.next() {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some(other) => value.push(other),
                    None => break,
                }
                continue;
            }
            if n == quote {
                break;
            }
            value.push(n);
        }
        out.push(value);
    }
    out
}

fn parse_claude(line: &str) -> Vec<StreamEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    let edges = turn_edges(&value);
    if !edges.is_empty() {
        return edges;
    }
    if value["type"] == "system" {
        if value["subtype"] != "background_tasks_changed" {
            return vec![];
        }
        let pending = value["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|task| Background {
                kind: task["task_type"].as_str().unwrap_or("").to_owned(),
                description: task["description"].as_str().unwrap_or("").to_owned(),
            })
            .collect();
        return vec![StreamEvent::Background(pending)];
    }
    thread::items_of(&value)
        .into_iter()
        .map(StreamEvent::Step)
        .collect()
}

fn parse_cursor(line: &str) -> Vec<StreamEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    let edges = turn_edges(&value);
    if !edges.is_empty() {
        return edges;
    }
    match value["type"].as_str() {
        Some("thinking") if value["subtype"] == "delta" => {
            let text = value["text"].as_str().unwrap_or("").to_owned();
            if text.is_empty() {
                vec![]
            } else {
                vec![StreamEvent::Step(Item::Thinking {
                    id: String::new(),
                    at: String::new(),
                    text,
                })]
            }
        }
        Some("tool_call") if value["subtype"] == "started" => {
            cursor_tool(&value, false).into_iter().collect()
        }
        Some("tool_call") if value["subtype"] == "completed" => {
            cursor_tool(&value, true).into_iter().collect()
        }
        Some("assistant") => {
            let text = value["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("");
            if text.trim().is_empty() {
                vec![]
            } else {
                vec![StreamEvent::Step(Item::Text {
                    id: "assistant".into(),
                    at: String::new(),
                    text,
                })]
            }
        }
        _ => vec![],
    }
}

fn cursor_tool(value: &Value, completed: bool) -> Option<StreamEvent> {
    let call = value["tool_call"].as_object()?;
    let (key, body) = call.iter().find(|(key, _)| key.ends_with("ToolCall"))?;
    let name = key.strip_suffix("ToolCall").unwrap_or(key).to_owned();
    let id = value["call_id"]
        .as_str()
        .or(body["toolCallId"].as_str())
        .unwrap_or("")
        .to_owned();
    let command = body["args"]["command"].as_str().unwrap_or("").to_owned();
    let summary = if command.is_empty() {
        body["args"]["description"]
            .as_str()
            .or(body["description"].as_str())
            .unwrap_or("")
            .to_owned()
    } else {
        command.clone()
    };
    if !completed {
        return Some(StreamEvent::Step(Item::Tool {
            id,
            at: String::new(),
            name,
            summary: summary.clone(),
            detail: summary,
            args: vec![],
        }));
    }
    let stdout = body["result"]["success"]["stdout"]
        .as_str()
        .or(body["result"]["success"]["interleavedOutput"].as_str())
        .unwrap_or("");
    let error = body["result"].get("error").is_some()
        || body["result"]["success"]["exitCode"]
            .as_i64()
            .is_some_and(|code| code != 0);
    Some(StreamEvent::Step(Item::Result {
        tool: id,
        at: String::new(),
        error,
        text: stdout.to_owned(),
        links: vec![],
    }))
}

/// `codex exec --json` events, from the documented schema. A live CLI was not available to capture them.
fn parse_codex(line: &str) -> Vec<StreamEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    match value["type"].as_str() {
        Some("thread.started") => value["thread_id"]
            .as_str()
            .map(|id| vec![StreamEvent::Conversation(id.to_owned())])
            .unwrap_or_default(),
        Some("item.started") => codex_item(&value["item"], false),
        Some("item.completed") => codex_item(&value["item"], true),
        Some("turn.completed") => vec![StreamEvent::TurnEnd {
            text: String::new(),
            error: None,
        }],
        Some("turn.failed") => {
            let message = value["error"]["message"]
                .as_str()
                .or(value["error"].as_str())
                .unwrap_or("the agent failed");
            vec![StreamEvent::TurnEnd {
                text: String::new(),
                error: Some(message.to_owned()),
            }]
        }
        Some("error") => {
            let message = value["message"].as_str().unwrap_or("the agent failed");
            vec![StreamEvent::TurnEnd {
                text: String::new(),
                error: Some(message.to_owned()),
            }]
        }
        _ => vec![],
    }
}

fn codex_item(item: &Value, completed: bool) -> Vec<StreamEvent> {
    let id = item["id"].as_str().unwrap_or("").to_owned();
    match item["type"].as_str() {
        Some("reasoning") if completed => text_step(&id, item["text"].as_str().unwrap_or(""), true),
        Some("agent_message") if completed => {
            text_step(&id, item["text"].as_str().unwrap_or(""), false)
        }
        Some("command_execution") if !completed => {
            let command = item["command"].as_str().unwrap_or("").to_owned();
            vec![StreamEvent::Step(Item::Tool {
                id,
                at: String::new(),
                name: "Bash".into(),
                summary: command.clone(),
                detail: command,
                args: vec![],
            })]
        }
        Some("command_execution") => {
            let output = item["aggregated_output"].as_str().unwrap_or("").to_owned();
            let error = item["exit_code"].as_i64().is_some_and(|code| code != 0);
            vec![StreamEvent::Step(Item::Result {
                tool: id,
                at: String::new(),
                error,
                text: output,
                links: vec![],
            })]
        }
        Some("file_change") if completed => {
            let summary = item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|change| change["path"].as_str())
                .collect::<Vec<_>>()
                .join(", ");
            vec![StreamEvent::Step(Item::Tool {
                id,
                at: String::new(),
                name: "file_change".into(),
                summary: summary.clone(),
                detail: summary,
                args: vec![],
            })]
        }
        _ => vec![],
    }
}

fn text_step(id: &str, text: &str, thinking: bool) -> Vec<StreamEvent> {
    if text.is_empty() {
        return vec![];
    }
    let item = if thinking {
        Item::Thinking {
            id: id.into(),
            at: String::new(),
            text: text.into(),
        }
    } else {
        Item::Text {
            id: id.into(),
            at: String::new(),
            text: text.into(),
        }
    };
    vec![StreamEvent::Step(item)]
}

fn turn_edges(value: &Value) -> Vec<StreamEvent> {
    if value["type"] == "result" {
        let text = value["result"].as_str().unwrap_or("").to_owned();
        let error = (value["is_error"].as_bool() == Some(true)).then(|| {
            if text.is_empty() {
                "the agent failed".into()
            } else {
                text.clone()
            }
        });
        return vec![StreamEvent::TurnEnd { text, error }];
    }
    if value["type"] == "system" && value["subtype"] == "init" {
        let conversation = value["session_id"].as_str().map(|id| StreamEvent::Conversation(id.to_owned()));
        return conversation.into_iter().chain([StreamEvent::TurnStart]).collect();
    }
    vec![]
}

#[cfg(test)]
pub(crate) struct Outcome {
    pub(crate) conversation: Option<String>,
    pub(crate) steps: Vec<Item>,
    pub(crate) text: Option<String>,
    pub(crate) done: bool,
    pub(crate) error: Option<String>,
}

#[cfg(test)]
pub(crate) fn replay(provider: Provider, lines: &str) -> Outcome {
    let mut outcome = Outcome {
        conversation: None,
        steps: vec![],
        text: None,
        done: false,
        error: None,
    };
    for line in lines.lines() {
        for event in (driver(provider).parse)(line) {
            match event {
                StreamEvent::Conversation(id) => outcome.conversation = Some(id),
                StreamEvent::Step(item) => {
                    if !outcome.done {
                        if let Item::Text { text, .. } = &item {
                            outcome.text = Some(text.clone());
                        }
                    }
                    push_step(&mut outcome.steps, item);
                }
                StreamEvent::TurnEnd { text, error } => {
                    if !text.is_empty() {
                        outcome.text = Some(text);
                    }
                    outcome.error = error;
                    outcome.done = true;
                }
                StreamEvent::TurnStart | StreamEvent::Background(_) => {}
            }
        }
    }
    outcome
}

pub(crate) fn push_step(steps: &mut Vec<Item>, item: Item) {
    if let Item::Thinking { text: more, .. } = &item {
        if let Some(Item::Thinking { text, .. }) = steps.last_mut() {
            if !text.is_empty() && !more.is_empty() {
                text.push(' ');
            }
            text.push_str(more);
            let clipped = thread::clip(text, STEP_CHARS);
            *text = clipped;
            return;
        }
    }
    steps.push(clip_item(item));
    let extra = steps.len().saturating_sub(STEP_KEEP);
    if extra > 0 {
        steps.drain(..extra);
    }
}

fn clip_item(item: Item) -> Item {
    match item {
        Item::Text { id, at, text } => Item::Text {
            id,
            at,
            text: thread::clip(&text, STEP_CHARS),
        },
        Item::Thinking { id, at, text } => Item::Thinking {
            id,
            at,
            text: thread::clip(&text, STEP_CHARS),
        },
        Item::Tool {
            id,
            at,
            name,
            summary,
            detail,
            args,
        } => Item::Tool {
            id,
            at,
            name,
            summary: thread::clip(&summary, STEP_CHARS),
            detail: thread::clip(&detail, STEP_CHARS),
            args,
        },
        Item::Result {
            tool,
            at,
            error,
            text,
            links,
        } => Item::Result {
            tool,
            at,
            error,
            text: thread::clip(&text, STEP_CHARS),
            links,
        },
        other => other,
    }
}

pub struct ChildSpec<'a> {
    pub bin: &'a Path,
    pub argv: &'a [String],
    pub cwd: &'a Path,
    pub env: &'a [(String, String)],
    pub path: Option<&'a str>,
    pub strip: &'a [&'a str],
}

pub fn spawn(
    spec: ChildSpec,
    on_event: impl Fn(ChildEvent) + Send + 'static,
) -> Result<u32, String> {
    if !spec.bin.is_file() {
        return Err(format!(
            "{} is not installed on this machine",
            spec.bin.display()
        ));
    }
    let mut cmd = crate::process::command(spec.bin);
    cmd.args(spec.argv)
        .current_dir(spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in spec.strip {
        cmd.env_remove(key);
    }
    cmd.env_remove(crate::agent::TAB_ENV);
    cmd.env_remove(crate::agent::TASK_ENV);
    for (key, value) in spec.env {
        cmd.env(key, value);
    }
    if let Some(path) = spec.path {
        cmd.env("PATH", path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", spec.bin.display()))?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or("the child has no stdout")?;
    let mut stderr = child.stderr.take().ok_or("the child has no stderr")?;
    let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stored = stderr_buf.clone();
    let (closed, stderr_closed) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut chunk = [0; 4096];
        loop {
            match stderr.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    let mut buf = stored.lock().unwrap();
                    buf.extend_from_slice(&chunk[..read]);
                    let extra = buf.len().saturating_sub(4096);
                    buf.drain(..extra);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let _ = closed.send(());
    });
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            on_event(ChildEvent::Line(line));
        }
        let _ = stderr_closed.recv_timeout(STDERR_CLOSE_WAIT);
        let code = child.wait().ok().and_then(|status| status.code());
        let stderr = String::from_utf8_lossy(&stderr_buf.lock().unwrap())
            .trim()
            .to_owned();
        on_event(ChildEvent::Exited { code, stderr });
    });
    Ok(pid)
}

pub fn kill(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = crate::process::command("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();
    }
}

/// Claude Code and the Cursor CLI start a background shell in a session of its own, so the agent's
/// process group alone does not reach it.
pub fn kill_tree(pid: u32) {
    #[cfg(unix)]
    let targets = signal_targets_under(pid);
    kill(pid);
    #[cfg(unix)]
    signal(&targets);
}

#[cfg(unix)]
pub fn kill_descendants(pid: u32) {
    signal(&signal_targets_under(pid));
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pid: i32,
    started: String,
}

/// A shell the agent started in a session of its own is reparented when the agent exits, and only this list still names it.
pub fn seen_under(pid: u32, before: &[Process]) -> Vec<Process> {
    #[cfg(unix)]
    {
        let under: Vec<i32> = signal_targets_under(pid).into_iter().filter(|target| *target > 0).collect();
        let mut ask: Vec<i32> = under.iter().copied().chain(before.iter().map(|seen| seen.pid)).collect();
        ask.sort_unstable();
        ask.dedup();
        recorded(&under, before, running(&ask))
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        before.to_vec()
    }
}

#[cfg(unix)]
fn recorded(under: &[i32], before: &[Process], now: Vec<Running>) -> Vec<Process> {
    if now.is_empty() {
        return before.to_vec();
    }
    now.into_iter().map(|now| now.process).filter(|now| under.contains(&now.pid) || before.contains(now)).collect()
}

pub fn kill_seen(seen: &[Process]) {
    #[cfg(unix)]
    {
        let pids: Vec<i32> = seen.iter().map(|seen| seen.pid).collect();
        for now in running(&pids) {
            let pid = now.process.pid;
            if seen.contains(&now.process) {
                signal(&if now.pgid == pid { vec![-pid, pid] } else { vec![pid] });
            }
        }
    }
    #[cfg(not(unix))]
    let _ = seen;
}

#[cfg(unix)]
fn signal(targets: &[i32]) {
    for &target in targets {
        unsafe {
            libc::kill(target, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
fn signal_targets_under(root: u32) -> Vec<i32> {
    targets_under(&process_table(), root)
}

#[cfg(unix)]
fn ps(args: &[&str]) -> String {
    crate::process::command("/bin/ps")
        .args(args)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

#[cfg(unix)]
struct Row {
    pid: u32,
    ppid: u32,
    pgid: u32,
}

#[cfg(unix)]
fn process_table() -> Vec<Row> {
    ps(&["-A", "-o", "pid=,ppid=,pgid="])
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().map(|field| field.parse().ok());
            Some(Row { pid: fields.next()??, ppid: fields.next()??, pgid: fields.next()?? })
        })
        .collect()
}

#[cfg(unix)]
struct Running {
    process: Process,
    pgid: i32,
}

#[cfg(unix)]
fn running(pids: &[i32]) -> Vec<Running> {
    if pids.is_empty() {
        return vec![];
    }
    let list = pids.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
    ps(&["-o", "pid=,pgid=,lstart=", "-p", &list])
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let pgid = fields.next()?.parse().ok()?;
            Some(Running { process: Process { pid, started: fields.collect::<Vec<_>>().join(" ") }, pgid })
        })
        .collect()
}

#[cfg(unix)]
fn targets_under(rows: &[Row], root: u32) -> Vec<i32> {
    let mut below = vec![];
    let mut parents = vec![root];
    let mut targets = vec![];
    while let Some(parent) = parents.pop() {
        for &Row { pid, ppid, pgid } in rows {
            if pid > 1 && ppid == parent && pid != root && !below.contains(&pid) {
                below.push(pid);
                parents.push(pid);
                if pid == pgid {
                    targets.push(-(pid as i32));
                }
                targets.push(pid as i32);
            }
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::{codex_notify_flag, codex_user_notify};
    #[cfg(unix)]
    use crate::delegate::tests::{scratch, script, stopped};
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    #[cfg(unix)]
    fn within(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while !done() {
            if start.elapsed() > limit {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    #[test]
    #[cfg(unix)]
    fn a_recorded_pid_is_killed_only_while_it_names_the_same_process() {
        let mut child = crate::process::command("sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        super::kill_seen(&[super::Process { pid, started: "Thu Jan 1 00:00:00 1970".into() }]);
        let reused = !within(Duration::from_millis(500), || child.try_wait().unwrap().is_some());
        let seen: Vec<super::Process> = super::running(&[pid]).into_iter().map(|now| now.process).collect();
        super::kill_seen(&seen);
        let same = within(Duration::from_secs(3), || child.try_wait().unwrap().is_some());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!((reused, same), (true, true), "(survived a record with another start time, killed with its own start time)");
    }

    #[test]
    #[cfg(unix)]
    fn a_shell_recorded_before_it_was_reparented_is_still_killed() {
        use std::os::unix::process::CommandExt;
        let dir = scratch("reparented");
        let (pidfile, release) = (dir.join("shell.pid"), dir.join("release"));
        let middle = script(
            &dir,
            "middle",
            &format!(
                "perl -e 'use POSIX (); POSIX::setsid(); exec @ARGV' sleep 30 &\necho $! > '{}'\nwhile [ ! -e '{}' ]; do sleep 0.05; done",
                pidfile.display(),
                release.display()
            ),
        );
        let agent = script(&dir, "agent", &format!("'{}' &\nexec sleep 30", middle.display()));
        let mut agent = crate::process::command(&agent).process_group(0).spawn().unwrap();
        let root = agent.id();
        assert!(within(Duration::from_secs(5), || std::fs::read_to_string(&pidfile).is_ok_and(|text| text.ends_with('\n'))), "the shell never started");
        let shell: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        let first = super::seen_under(root, &[]);
        std::fs::write(&release, "").unwrap();
        let reparented = within(Duration::from_secs(5), || !super::targets_under(&super::process_table(), root).contains(&shell));
        let second = super::seen_under(root, &first);
        super::kill_seen(&second);
        let killed = stopped(shell);
        unsafe {
            libc::kill(-(root as i32), libc::SIGKILL);
            libc::kill(shell, libc::SIGKILL);
        }
        let _ = agent.wait();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!((reparented, killed), (true, true), "(the shell left the agent's tree, kill_seen stopped it)");
    }

    #[test]
    #[cfg(unix)]
    fn a_ps_that_prints_nothing_keeps_what_was_recorded() {
        let process = |pid, started: &str| super::Process { pid, started: started.into() };
        let shell = process(4242, "Sat Oct 31 19:52:52 2026");
        let fresh = super::Running { process: process(4343, "Sat Oct 31 20:00:00 2026"), pgid: 4343 };
        assert_eq!(
            [super::recorded(&[], &[shell.clone()], vec![]), super::recorded(&[4343], &[shell], vec![fresh])],
            [vec![process(4242, "Sat Oct 31 19:52:52 2026")], vec![process(4343, "Sat Oct 31 20:00:00 2026")]],
            "an empty ps keeps the list, and a ps that answers drops the shell it no longer lists"
        );
    }

    #[cfg(unix)]
    const RECORD_PID: &str = "PINNED_TEST_RECORD_PID";

    #[test]
    #[cfg(unix)]
    fn a_process_recorded_under_another_timezone_and_locale_is_still_killed() {
        let mut child = crate::process::command("sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        let out = crate::process::command(std::env::current_exe().unwrap())
            .args(["--exact", "delegate::drivers::tests::record_a_start_time", "--ignored", "--nocapture", "--test-threads=1"])
            .env(RECORD_PID, pid.to_string())
            .env("TZ", "JST-9")
            .env("LC_ALL", "ja_JP.UTF-8")
            .output()
            .unwrap();
        let started = String::from_utf8_lossy(&out.stdout).lines().find_map(|line| line.split_once("started=")).map(|(_, started)| started.to_owned()).unwrap_or_default();
        let now: Vec<super::Process> = super::running(&[pid]).into_iter().map(|now| now.process).collect();
        super::kill_seen(&[super::Process { pid, started: started.clone() }]);
        let killed = within(Duration::from_secs(3), || child.try_wait().unwrap().is_some());
        let _ = child.kill();
        let _ = child.wait();
        assert!(killed, "recorded {started:?} under TZ=JST-9, and pinned now reads {now:?}");
    }

    #[test]
    #[cfg(unix)]
    #[ignore = "a_process_recorded_under_another_timezone_and_locale_is_still_killed runs it in a child process"]
    fn record_a_start_time() {
        let Some(pid) = std::env::var(RECORD_PID).ok().and_then(|pid| pid.parse().ok()) else { return };
        for now in super::running(&[pid]) {
            println!("started={}", now.process.started);
        }
    }

    #[test]
    #[cfg(unix)]
    fn the_walk_never_targets_pid_0_or_1() {
        let rows = [(0, 500, 0), (1, 500, 1), (501, 500, 501), (502, 501, 501)].map(|(pid, ppid, pgid)| super::Row { pid, ppid, pgid });
        assert_eq!(super::targets_under(&rows, 500), vec![-501, 501, 502]);
    }

    #[test]
    fn a_top_level_notify_array_is_read_and_a_table_one_is_not() {
        let text = "notify = [\n  \"/bin/echo\",\n  \"hi\",\n]\n[tui]\nnotify = [\"nope\"]\n";
        assert_eq!(codex_user_notify(text), vec!["/bin/echo".to_owned(), "hi".to_owned()]);
        assert_eq!(codex_user_notify("model = \"gpt-5.4\"\n"), Vec::<String>::new());
        assert_eq!(codex_user_notify("notify = \"/bin/echo\"\n"), Vec::<String>::new());
    }

    #[test]
    fn a_notify_flag_escapes_a_tab_in_a_user_argument() {
        let flag = codex_notify_flag(std::path::Path::new("/bin/pinned"), &["say\thi".into()]);
        let text = flag.join(" ");
        assert!(text.contains("say\\thi"), "{text}");
        assert!(!text.contains('\t'), "{text}");
    }
}
