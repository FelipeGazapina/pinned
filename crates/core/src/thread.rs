use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const PREVIEW_CHARS: usize = 4000;
const PLAN_CHARS: usize = 40_000;
const SUMMARY_CHARS: usize = 300;
/// The biggest image, as base64, that a thread page carries inline.
const PICTURE_CHARS: usize = 2 << 20;
const READ_LIMIT: u64 = 8 << 20;

#[derive(Serialize, Debug, PartialEq, Clone)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Item {
    /// What the user typed, or a slash command as `/name args`. `pictures` holds the `data:` URLs of
    /// the attached images small enough to send; `images` counts them all.
    User {
        id: String,
        at: String,
        text: String,
        images: u32,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pictures: Vec<String>,
    },
    /// Markdown the assistant wrote.
    Text { id: String, at: String, text: String },
    /// The assistant's reasoning before it answered or used a tool.
    Thinking { id: String, at: String, text: String },
    /// A request to the model that failed, such as `API Error: 529 Overloaded`.
    Error { id: String, at: String, text: String },
    Tool { id: String, at: String, name: String, summary: String, detail: String },
    /// The outcome of the `Tool` whose id is `tool`.
    Result { tool: String, at: String, error: bool, text: String },
    /// Output of a local command such as `/model`, which claude shows without a turn.
    Note { id: String, at: String, text: String },
    /// A pull request the session opened or worked on.
    Pr { number: u64, url: String, repo: String },
    /// The model the session runs from here on, by its marketing name, such as `Haiku 5.5`.
    Model { name: String },
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Page {
    pub items: Vec<Item>,
    /// Where the next read starts.
    pub next: u64,
    /// The transcript was replaced, so these items start it over rather than follow the last read.
    pub reset: bool,
}

pub fn find(projects: &Path, session: &str) -> Option<PathBuf> {
    if session.is_empty() || !session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    let name = format!("{session}.jsonl");
    std::fs::read_dir(projects).ok()?.flatten().map(|dir| dir.path().join(&name)).find(|path| path.is_file())
}

/// The items in the whole lines of `file` from byte `from` on.
pub fn read(file: &Path, from: u64) -> std::io::Result<Page> {
    let mut f = std::fs::File::open(file)?;
    let len = f.metadata()?.len();
    let replaced = from > len;
    let from = if replaced { 0 } else { from };
    f.seek(SeekFrom::Start(from))?;
    let mut bytes = vec![];
    (&mut f).take(READ_LIMIT).read_to_end(&mut bytes)?;
    let whole = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let mut items: Vec<Item> = bytes[..whole].split(|b| *b == b'\n').filter_map(|line| serde_json::from_slice::<Value>(line).ok()).flat_map(|v| items_of(&v)).collect();
    let mut next = from + whole as u64;
    if whole == 0 && bytes.len() as u64 == READ_LIMIT {
        if let Some(rest) = rest_of_line(&mut f)? {
            next = from + READ_LIMIT + rest;
            items.push(Item::Note { id: format!("skipped:{from}"), at: String::new(), text: "An entry too large to show was left out.".into() });
        }
    }
    Ok(Page { items, next, reset: replaced })
}

/// How many more bytes the current line runs through its newline, or none while it is still being written.
fn rest_of_line(f: &mut impl Read) -> std::io::Result<Option<u64>> {
    let mut chunk = vec![0; 1 << 16];
    let mut skipped = 0;
    loop {
        let n = f.read(&mut chunk)?;
        if n == 0 {
            return Ok(None);
        }
        if let Some(i) = chunk[..n].iter().position(|b| *b == b'\n') {
            return Ok(Some(skipped + i as u64 + 1));
        }
        skipped += n as u64;
    }
}

fn items_of(line: &Value) -> Vec<Item> {
    let str_of = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    if line["isSidechain"].as_bool() == Some(true) || line["isMeta"].as_bool() == Some(true) {
        return vec![];
    }
    let id = str_of(&line["uuid"]);
    let at = str_of(&line["timestamp"]);
    match line["type"].as_str() {
        Some("pr-link") => match line["prNumber"].as_u64() {
            Some(number) => vec![Item::Pr { number, url: str_of(&line["prUrl"]), repo: str_of(&line["prRepository"]) }],
            None => vec![],
        },
        Some("user") if line["isCompactSummary"].as_bool() == Some(true) => vec![Item::Note { id, at, text: "The conversation was compacted.".into() }],
        Some("user") => user_items(&line["message"]["content"], id, at),
        Some("attachment") if line["attachment"]["type"] == "model" => match line["attachment"]["identity"]["marketingName"].as_str() {
            Some(name) => vec![Item::Model { name: name.to_owned() }],
            None => vec![],
        },
        Some("assistant") if line["isApiErrorMessage"].as_bool() == Some(true) => {
            let text = line["message"]["content"].as_array().into_iter().flatten().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n");
            vec![Item::Error { id, at, text }]
        }
        Some("assistant") => {
            let blocks = line["message"]["content"].as_array().cloned().unwrap_or_default();
            blocks
                .iter()
                .enumerate()
                .filter_map(|(i, block)| {
                    let id = format!("{id}:{i}");
                    match block["type"].as_str()? {
                        "text" => {
                            let text = str_of(&block["text"]);
                            (!text.trim().is_empty()).then(|| Item::Text { id, at: at.clone(), text })
                        }
                        "thinking" => {
                            let text = str_of(&block["thinking"]);
                            (!text.trim().is_empty()).then(|| Item::Thinking { id, at: at.clone(), text })
                        }
                        "tool_use" => {
                            let name = str_of(&block["name"]);
                            let (summary, detail) = describe_tool(&name, &block["input"]);
                            Some(Item::Tool { id: str_of(&block["id"]), at: at.clone(), name, summary, detail })
                        }
                        _ => None,
                    }
                })
                .collect()
        }
        _ => vec![],
    }
}

fn user_items(content: &Value, id: String, at: String) -> Vec<Item> {
    if let Some(text) = content.as_str() {
        return match spoken(text) {
            Some(Spoken::Prompt(text)) => vec![Item::User { id, at, text, images: 0, pictures: vec![] }],
            Some(Spoken::Note(text)) => vec![Item::Note { id, at, text }],
            None => vec![],
        };
    }
    let blocks = content.as_array().cloned().unwrap_or_default();
    let results: Vec<Item> = blocks
        .iter()
        .filter(|b| b["type"] == "tool_result")
        .map(|b| Item::Result {
            tool: b["tool_use_id"].as_str().unwrap_or_default().to_owned(),
            at: at.clone(),
            error: b["is_error"].as_bool() == Some(true),
            text: clip(&result_text(&b["content"]), PREVIEW_CHARS),
        })
        .collect();
    let text = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n");
    let images: Vec<&Value> = blocks.iter().filter(|b| b["type"] == "image").collect();
    let pictures = images.iter().filter_map(|b| picture(&b["source"])).collect();
    let images = images.len() as u32;
    let said = match spoken(&text) {
        Some(Spoken::Prompt(text)) => Some(Item::User { id, at, text, images, pictures }),
        Some(Spoken::Note(text)) => Some(Item::Note { id, at, text }),
        None if images > 0 && results.is_empty() => Some(Item::User { id, at, text: String::new(), images, pictures }),
        None => None,
    };
    results.into_iter().chain(said).collect()
}

/// A `data:` URL for an image block's base64 source, when it is a picture a page can show and small enough.
fn picture(source: &Value) -> Option<String> {
    let kind = source["media_type"].as_str()?;
    let data = source["data"].as_str()?;
    let shown = ["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&kind);
    let base64 = data.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
    (source["type"] == "base64" && shown && base64 && data.len() <= PICTURE_CHARS).then(|| format!("data:{kind};base64,{data}"))
}

enum Spoken {
    Prompt(String),
    Note(String),
}

/// Claude Code wraps the slash commands it runs and the messages it injects in tags, and so do the
/// apps around it: `<system-reminder>`, `<task-notification>` and more. A paste stays wrapped in
/// `<pasted_content>`, whose text is the user's.
fn spoken(text: &str) -> Option<Spoken> {
    let text = text.trim();
    if let Some(name) = tag(text, "command-name") {
        let args = tag(text, "command-args").unwrap_or_default();
        return Some(Spoken::Prompt(if args.is_empty() { name.to_owned() } else { format!("{name} {args}") }));
    }
    if let Some(out) = tag(text, "local-command-stdout") {
        return (!out.trim().is_empty()).then(|| Spoken::Note(out.trim().to_owned()));
    }
    if let Some(command) = tag(text, "bash-input") {
        return Some(Spoken::Prompt(format!("! {command}")));
    }
    if text.starts_with("[Request interrupted by user") {
        return Some(Spoken::Note("Interrupted.".into()));
    }
    let mut said = vec![];
    let mut rest = text;
    while let Some((name, inner, after)) = leading_element(rest) {
        if name == "pasted_content" {
            said.push(inner.trim());
        }
        rest = after.trim_start();
    }
    said.push(rest);
    let said = said.join("\n").trim().to_owned();
    (!said.is_empty()).then_some(Spoken::Prompt(said))
}

/// The tag name, inner text and remainder of the `<name …>…</name>` element `text` starts with.
fn leading_element(text: &str) -> Option<(&str, &str, &str)> {
    let name_len = text.strip_prefix('<')?.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))?;
    let name = &text[1..=name_len];
    let open_end = text.find('>')? + 1;
    let close = format!("</{name}>");
    let close_at = text[open_end..].find(&close)? + open_end;
    (!name.is_empty()).then(|| (name, &text[open_end..close_at], &text[close_at + close.len()..]))
}

fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let start = text.find(&format!("<{name}>"))? + name.len() + 2;
    let end = text[start..].find(&format!("</{name}>"))? + start;
    Some(text[start..end].trim())
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

fn describe_tool(name: &str, input: &Value) -> (String, String) {
    let field = |k: &str| input[k].as_str().unwrap_or_default();
    let summary = match name {
        "Bash" => field("description").to_owned(),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => field("file_path").to_owned(),
        "Grep" | "Glob" => field("pattern").to_owned(),
        "WebFetch" => field("url").to_owned(),
        "WebSearch" => field("query").to_owned(),
        "Agent" | "Task" => field("description").to_owned(),
        "Skill" => field("skill").to_owned(),
        "AskUserQuestion" => input["questions"][0]["question"].as_str().unwrap_or_default().to_owned(),
        _ => String::new(),
    };
    let summary = if summary.is_empty() { first_string(input).unwrap_or_default() } else { summary };
    let detail = match name {
        "Bash" => field("command").to_owned(),
        "Edit" => format!("{}\n---\n{}", field("old_string"), field("new_string")),
        "Write" => field("content").to_owned(),
        "Agent" | "Task" => field("prompt").to_owned(),
        "ExitPlanMode" => return (clip(summary.lines().next().unwrap_or_default(), SUMMARY_CHARS), clip(field("plan"), PLAN_CHARS)),
        _ => String::new(),
    };
    (clip(summary.lines().next().unwrap_or_default(), SUMMARY_CHARS), clip(&detail, PREVIEW_CHARS))
}

fn first_string(input: &Value) -> Option<String> {
    input.as_object()?.values().find_map(|v| v.as_str()).map(str::to_owned)
}

fn clip(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn page(lines: &[&str]) -> Page {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        read(file.path(), 0).unwrap()
    }

    #[test]
    fn a_turn_reads_as_prompt_tool_result_and_answer() {
        let items = page(&[
            r#"{"type":"user","uuid":"u1","timestamp":"2026-10-23T10:00:00Z","message":{"role":"user","content":"fix the bug"}}"#,
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-10-23T10:00:05Z","message":{"content":[{"type":"thinking","thinking":""},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test","description":"Run the tests"}}]}}"#,
            r#"{"type":"user","uuid":"u2","timestamp":"2026-10-23T10:00:09Z","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok","is_error":false}]}}"#,
            r#"{"type":"assistant","uuid":"a2","timestamp":"2026-10-23T10:01:00Z","message":{"content":[{"type":"text","text":"Fixed."}]}}"#,
            r#"{"type":"pr-link","prNumber":7,"prUrl":"https://github.com/o/r/pull/7","prRepository":"o/r"}"#,
            r#"{"type":"attachment","attachment":{"type":"model","identity":{"modelId":"claude-haiku-5-5","marketingName":"Haiku 5.5"}}}"#,
            r#"{"type":"attachment","attachment":{"type":"hook_success","content":"x"}}"#,
        ])
        .items;
        assert_eq!(
            items,
            vec![
                Item::User { id: "u1".into(), at: "2026-10-23T10:00:00Z".into(), text: "fix the bug".into(), images: 0, pictures: vec![] },
                Item::Tool { id: "t1".into(), at: "2026-10-23T10:00:05Z".into(), name: "Bash".into(), summary: "Run the tests".into(), detail: "cargo test".into() },
                Item::Result { tool: "t1".into(), at: "2026-10-23T10:00:09Z".into(), error: false, text: "ok".into() },
                Item::Text { id: "a2:0".into(), at: "2026-10-23T10:01:00Z".into(), text: "Fixed.".into() },
                Item::Pr { number: 7, url: "https://github.com/o/r/pull/7".into(), repo: "o/r".into() },
                Item::Model { name: "Haiku 5.5".into() },
            ]
        );
    }

    #[test]
    fn injected_lines_are_hidden_and_commands_shown_by_name() {
        let items = page(&[
            r#"{"type":"user","uuid":"m","isMeta":true,"message":{"content":"<local-command-caveat>x</local-command-caveat>"}}"#,
            r#"{"type":"user","uuid":"c","message":{"content":"<command-message>goal</command-message>\n<command-name>/goal</command-name>\n<command-args>ship it</command-args>"}}"#,
            r#"{"type":"user","uuid":"o","message":{"content":"<local-command-stdout>Goal set: ship it</local-command-stdout>"}}"#,
            r#"{"type":"user","uuid":"n","message":{"content":"<task-notification>done</task-notification>"}}"#,
            r#"{"type":"user","uuid":"s","isSidechain":true,"message":{"content":"subagent prompt"}}"#,
            r#"{"type":"api-request-blob","message":{"role":"user","content":"huge"}}"#,
            r#"{"type":"user","uuid":"i","message":{"content":[{"type":"text","text":"look [Image #1]"},{"type":"image","source":{}}]}}"#,
        ])
        .items;
        assert_eq!(
            items,
            vec![
                Item::User { id: "c".into(), at: "".into(), text: "/goal ship it".into(), images: 0, pictures: vec![] },
                Item::Note { id: "o".into(), at: "".into(), text: "Goal set: ship it".into() },
                Item::User { id: "i".into(), at: "".into(), text: "look [Image #1]".into(), images: 1, pictures: vec![] },
            ]
        );
    }

    #[test]
    fn a_prompt_that_starts_with_markup_shows_and_injected_tags_do_not() {
        let items = page(&[
            r#"{"type":"user","uuid":"a","message":{"content":"<Button> doesn't re-render"}}"#,
            r#"{"type":"user","uuid":"b","message":{"content":"<system-reminder>be brief</system-reminder>\n<ci-monitor-event kind=\"x\">red</ci-monitor-event>"}}"#,
            r#"{"type":"user","uuid":"c","message":{"content":"<pasted_content id=\"1\">\nlog line\n</pasted_content>\nwhy does this fail?"}}"#,
            r#"{"type":"user","uuid":"d","message":{"content":"<bash-input>ls</bash-input>"}}"#,
            r#"{"type":"user","uuid":"e","message":{"content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#,
            r#"{"type":"user","uuid":"f","isCompactSummary":true,"message":{"content":"This session is being continued from a previous conversation."}}"#,
            r#"{"type":"user","uuid":"g","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"denied"},{"type":"text","text":"use the other folder"}]}}"#,
        ])
        .items;
        assert_eq!(
            items,
            vec![
                Item::User { id: "a".into(), at: "".into(), text: "<Button> doesn't re-render".into(), images: 0, pictures: vec![] },
                Item::User { id: "c".into(), at: "".into(), text: "log line\nwhy does this fail?".into(), images: 0, pictures: vec![] },
                Item::User { id: "d".into(), at: "".into(), text: "! ls".into(), images: 0, pictures: vec![] },
                Item::Note { id: "e".into(), at: "".into(), text: "Interrupted.".into() },
                Item::Note { id: "f".into(), at: "".into(), text: "The conversation was compacted.".into() },
                Item::Result { tool: "t1".into(), at: "".into(), error: false, text: "denied".into() },
                Item::User { id: "g".into(), at: "".into(), text: "use the other folder".into(), images: 0, pictures: vec![] },
            ]
        );
    }

    #[test]
    fn thinking_errors_plans_questions_and_pictures_are_kept() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let items = page(&[
            &format!(r#"{{"type":"user","uuid":"u","timestamp":"t0","message":{{"role":"user","content":[{{"type":"text","text":"what is this?"}},{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{png}"}}}},{{"type":"image","source":{{"type":"base64","media_type":"image/svg+xml","data":"PHN2Zz4="}}}}]}}}}"#),
            r#"{"type":"assistant","uuid":"a","timestamp":"t1","message":{"content":[{"type":"thinking","thinking":"Found the page. I'll read it now.\n\n","signature":"CAQS"},{"type":"tool_use","id":"q1","name":"AskUserQuestion","input":{"questions":[{"question":"Which one should open the PR?","header":"PR","multiSelect":false,"options":[{"label":"This session","description":"x"},{"label":"The other session","description":"y"}]}]}}]}}"#,
            r#"{"type":"user","uuid":"r","timestamp":"t2","message":{"content":[{"tool_use_id":"q1","type":"tool_result","content":"Your questions have been answered: \"Which one should open the PR?\"=\"The other session\". You can now continue with these answers in mind."}]}}"#,
            r##"{"type":"assistant","uuid":"p","timestamp":"t3","message":{"content":[{"type":"tool_use","id":"p1","name":"ExitPlanMode","input":{"plan":"# Ship the fix\n\n1. Patch\n2. Test"}}]}}"##,
            r#"{"parentUuid":"p","isSidechain":false,"type":"assistant","uuid":"e","timestamp":"t4","isApiErrorMessage":true,"message":{"model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"API Error: Your computer went to sleep mid-response. The response above may be incomplete."}]}}"#,
        ])
        .items;
        assert_eq!(
            items,
            vec![
                Item::User { id: "u".into(), at: "t0".into(), text: "what is this?".into(), images: 2, pictures: vec![format!("data:image/png;base64,{png}")] },
                Item::Thinking { id: "a:0".into(), at: "t1".into(), text: "Found the page. I'll read it now.\n\n".into() },
                Item::Tool { id: "q1".into(), at: "t1".into(), name: "AskUserQuestion".into(), summary: "Which one should open the PR?".into(), detail: "".into() },
                Item::Result {
                    tool: "q1".into(),
                    at: "t2".into(),
                    error: false,
                    text: r#"Your questions have been answered: "Which one should open the PR?"="The other session". You can now continue with these answers in mind."#.into()
                },
                Item::Tool { id: "p1".into(), at: "t3".into(), name: "ExitPlanMode".into(), summary: "# Ship the fix".into(), detail: "# Ship the fix\n\n1. Patch\n2. Test".into() },
                Item::Error { id: "e".into(), at: "t4".into(), text: "API Error: Your computer went to sleep mid-response. The response above may be incomplete.".into() },
            ]
        );
    }

    #[test]
    fn new_fields_and_kinds_leave_the_old_shapes_alone() {
        let user = Item::User { id: "u".into(), at: "t".into(), text: "hi".into(), images: 0, pictures: vec![] };
        assert_eq!(serde_json::to_value(&user).unwrap(), serde_json::json!({ "kind": "user", "id": "u", "at": "t", "text": "hi", "images": 0 }));
        let thought = Item::Thinking { id: "a:0".into(), at: "t".into(), text: "hm".into() };
        assert_eq!(serde_json::to_value(&thought).unwrap(), serde_json::json!({ "kind": "thinking", "id": "a:0", "at": "t", "text": "hm" }));
    }

    #[test]
    fn a_session_id_that_is_not_a_plain_name_finds_nothing() {
        let projects = tempfile::tempdir().unwrap();
        let dir = projects.path().join("proj");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("ab-12.jsonl"), "").unwrap();
        std::fs::write(projects.path().join("outside.jsonl"), "").unwrap();
        assert_eq!(find(projects.path(), "ab-12"), Some(dir.join("ab-12.jsonl")));
        for session in ["../outside", "", "ab/../ab-12"] {
            assert_eq!(find(projects.path(), session), None, "{session:?}");
        }
    }

    #[test]
    fn a_line_past_the_read_limit_is_skipped_not_stuck_on() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let huge = format!(r#"{{"type":"user","uuid":"big","message":{{"content":"{}"}}}}"#, "x".repeat(READ_LIMIT as usize + 10));
        writeln!(file, "{huge}").unwrap();
        writeln!(file, r#"{{"type":"user","uuid":"after","message":{{"content":"after"}}}}"#).unwrap();
        let first = read(file.path(), 0).unwrap();
        assert_eq!(first.next, huge.len() as u64 + 1);
        assert!(matches!(first.items[..], [Item::Note { .. }]));
        assert_eq!(read(file.path(), first.next).unwrap().items, vec![Item::User { id: "after".into(), at: "".into(), text: "after".into(), images: 0, pictures: vec![] }]);

        let mut growing = tempfile::NamedTempFile::new().unwrap();
        write!(growing, "{}", &huge[..huge.len() - 2]).unwrap();
        assert_eq!(read(growing.path(), 0).unwrap(), Page { items: vec![], next: 0, reset: false }, "a line still being written waits");
    }

    #[test]
    fn reads_resume_after_the_last_whole_line() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let first = r#"{"type":"user","uuid":"u1","message":{"content":"one"}}"#;
        write!(file, "{first}\n{{\"type\":\"user\",\"uuid\":\"u2\",").unwrap();
        let page = read(file.path(), 0).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next, first.len() as u64 + 1);

        writeln!(file, r#""message":{{"content":"two"}}}}"#).unwrap();
        let rest = read(file.path(), page.next).unwrap();
        assert_eq!(rest.items, vec![Item::User { id: "u2".into(), at: "".into(), text: "two".into(), images: 0, pictures: vec![] }]);
        assert_eq!(read(file.path(), rest.next).unwrap().items, vec![]);
        let over = read(file.path(), 1 << 30).unwrap();
        assert_eq!((over.items.len(), over.reset), (2, true), "an offset past the end starts over");
        assert!(!rest.reset);
    }

    #[test]
    fn finds_the_transcript_in_any_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("-repo-a")).unwrap();
        std::fs::create_dir_all(dir.path().join("-repo-b")).unwrap();
        std::fs::write(dir.path().join("-repo-b/s1.jsonl"), "").unwrap();
        assert_eq!(find(dir.path(), "s1"), Some(dir.path().join("-repo-b/s1.jsonl")));
        assert_eq!(find(dir.path(), "s2"), None);
    }

    #[test]
    fn long_text_is_clipped_on_a_char_boundary() {
        assert_eq!(clip("ééé", 2), "éé…");
        assert_eq!(clip("ab", 2), "ab");
    }
}
