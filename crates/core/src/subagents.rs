use crate::thread::{self, Item};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How much of the end of an agent's transcript a card reads. A line longer than this leaves fewer steps.
const TAIL: u64 = 256 << 10;
const STEPS: usize = 6;
/// A card shows a glance at each step, and the page carries every agent on each poll.
const STEP_CHARS: usize = 1200;

#[derive(Serialize, Debug, PartialEq, Clone, Copy, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Running,
    Done,
    Stopped,
}

/// An agent claude spawned with its `Agent` tool, as of the end of its own transcript.
#[derive(Serialize, Debug, PartialEq, Clone, schemars::JsonSchema)]
pub struct Subagent {
    pub id: String,
    /// The id of the `Agent` tool use that spawned it, in the session's transcript or, for a nested
    /// agent, in its parent's.
    pub tool: String,
    /// The agent that spawned this one.
    pub parent: Option<String>,
    pub name: String,
    pub kind: String,
    pub model: Option<String>,
    pub state: State,
    /// When the agent's last message was written.
    pub updated: String,
    /// Its latest tools, thoughts and texts, oldest first, with the results of those tools.
    pub steps: Vec<Item>,
}

#[derive(Deserialize, Debug, PartialEq, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct Meta {
    agent_type: String,
    description: String,
    tool_use_id: String,
    parent_agent_id: Option<String>,
    model: Option<String>,
    /// Newer claude versions add it once the agent ends.
    finished: bool,
}

/// How the last message in an agent's transcript leaves it.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Last {
    Ended,
    Interrupted,
    Busy,
}

#[derive(Debug, PartialEq, Clone)]
struct Tail {
    last: Last,
    updated: String,
    steps: Vec<Item>,
}

/// `written` is when the agent's transcript last changed, and `process` when the tab's claude
/// started, while it runs.
fn state(finished: bool, last: Last, written: SystemTime, process: Option<SystemTime>) -> State {
    match (finished, last) {
        (true, _) | (false, Last::Ended) => State::Done,
        (false, Last::Interrupted) => State::Stopped,
        // An agent runs inside its claude process, so one a dead process left mid-step never ends.
        (false, Last::Busy) => match process {
            Some(started) if written >= started => State::Running,
            _ => State::Stopped,
        },
    }
}

/// The whole lines of `bytes`, the end of a transcript; `cut` drops the first, which the read
/// started partway into.
fn tail_of(bytes: &[u8], cut: bool) -> Tail {
    let start = if cut { bytes.iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| i + 1) } else { 0 };
    let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1).max(start);
    let lines: Vec<Value> = bytes[start..end].split(|b| *b == b'\n').filter_map(|line| serde_json::from_slice(line).ok()).collect();
    let message = lines.iter().rev().find(|v| matches!(v["type"].as_str(), Some("user" | "assistant")));
    let last = message.map_or(Last::Busy, last_of);
    let updated = message.and_then(|v| v["timestamp"].as_str()).unwrap_or_default().to_owned();
    let items: Vec<Item> = lines.iter().flat_map(thread::items_of).filter_map(glance).collect();
    let step = |i: &Item| !matches!(i, Item::Result { .. });
    let from = items.iter().enumerate().rev().filter(|(_, i)| step(i)).nth(STEPS - 1).map_or(0, |(at, _)| at);
    let kept = &items[from..];
    let tools: Vec<&str> = kept.iter().filter_map(|i| if let Item::Tool { id, .. } = i { Some(id.as_str()) } else { None }).collect();
    let steps = kept.iter().filter(|i| if let Item::Result { tool, .. } = i { tools.contains(&tool.as_str()) } else { true }).cloned().collect();
    Tail { last, updated, steps }
}

fn last_of(message: &Value) -> Last {
    if message["type"] == "assistant" {
        return match message["message"]["stop_reason"].as_str() {
            Some("end_turn" | "stop_sequence") => Last::Ended,
            _ => Last::Busy,
        };
    }
    let content = &message["message"]["content"];
    let texts = content.as_str().into_iter().chain(content.as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()));
    if texts.into_iter().any(|t| t.trim_start().starts_with("[Request interrupted")) {
        Last::Interrupted
    } else {
        Last::Busy
    }
}

/// A step as a card shows it, or none for what a card leaves out, such as the prompt the agent got.
fn glance(item: Item) -> Option<Item> {
    let clip = |text: String| thread::clip(&text, STEP_CHARS);
    Some(match item {
        Item::Text { id, at, text } => Item::Text { id, at, text: clip(text) },
        Item::Thinking { id, at, text } => Item::Thinking { id, at, text: clip(text) },
        Item::Tool { id, at, name, summary, detail, args } => Item::Tool { id, at, name, summary, detail: clip(detail), args },
        Item::Result { tool, at, error, text, links } => Item::Result { tool, at, error, text: clip(text), links },
        _ => return None,
    })
}

fn read_tail(file: &Path, len: u64) -> std::io::Result<Tail> {
    let mut f = std::fs::File::open(file)?;
    let from = len.saturating_sub(TAIL);
    f.seek(SeekFrom::Start(from))?;
    let mut bytes = vec![];
    f.take(len - from).read_to_end(&mut bytes)?;
    Ok(tail_of(&bytes, from > 0))
}

struct Seen {
    sizes: (u64, u64),
    born: SystemTime,
    meta: Meta,
    tail: Tail,
}

/// What a scan read of each agent, so an agent whose files kept their sizes costs a stat.
#[derive(Default)]
pub struct Cache(HashMap<PathBuf, Seen>);

/// Every agent in `dir`, a session's `subagents` folder, in the order they were spawned. `process`
/// is when the tab's claude started, while it runs.
pub fn scan(dir: &Path, cache: &mut Cache, process: Option<SystemTime>) -> Vec<Subagent> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut found = vec![];
    for entry in entries.flatten() {
        let meta_file = entry.path();
        let Some(id) = meta_file.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("agent-")?.strip_suffix(".meta.json")) else { continue };
        let id = id.to_owned();
        let log = dir.join(format!("agent-{id}.jsonl"));
        let Ok(meta_stat) = entry.metadata() else { continue };
        let log_stat = std::fs::metadata(&log).ok();
        let sizes = (log_stat.as_ref().map_or(0, |s| s.len()), meta_stat.len());
        let written = log_stat.as_ref().unwrap_or(&meta_stat).modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let cached = cache.0.get(&meta_file);
        // A meta file being rewritten doesn't parse, so the agent shows as last read until the next scan.
        let meta = cached.is_none_or(|seen| seen.sizes != sizes).then(|| std::fs::read(&meta_file).ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok())).flatten();
        if let Some(meta) = meta {
            let tail = log_stat.as_ref().and_then(|stat| read_tail(&log, stat.len()).ok()).unwrap_or(Tail { last: Last::Busy, updated: String::new(), steps: vec![] });
            let born = cached.map(|seen| seen.born).unwrap_or_else(|| log_stat.as_ref().unwrap_or(&meta_stat).created().unwrap_or(SystemTime::UNIX_EPOCH));
            cache.0.insert(meta_file.clone(), Seen { sizes, born, meta, tail });
        }
        let Some(seen) = cache.0.get(&meta_file) else { continue };
        let agent = Subagent {
            id,
            tool: seen.meta.tool_use_id.clone(),
            parent: seen.meta.parent_agent_id.clone(),
            name: seen.meta.description.clone(),
            kind: seen.meta.agent_type.clone(),
            model: seen.meta.model.clone(),
            state: state(seen.meta.finished, seen.tail.last, written, process),
            updated: seen.tail.updated.clone(),
            steps: seen.tail.steps.clone(),
        };
        found.push((seen.born, meta_file, agent));
    }
    cache.0.retain(|file, _| file.parent() != Some(dir) || found.iter().any(|(_, f, _)| f == file));
    found.sort_by(|a, b| (a.0, &a.2.id).cmp(&(b.0, &b.2.id)));
    found.into_iter().map(|(_, _, agent)| agent).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn an_agent_reads_done_stopped_or_running_from_its_meta_last_message_and_process() {
        let t = |s: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        let rows = [
            (true, Last::Busy, t(5), None, State::Done),
            (true, Last::Interrupted, t(5), Some(t(1)), State::Done),
            (false, Last::Interrupted, t(5), Some(t(1)), State::Stopped),
            (false, Last::Ended, t(5), None, State::Done),
            (false, Last::Busy, t(5), Some(t(1)), State::Running),
            (false, Last::Busy, t(5), Some(t(5)), State::Running),
            (false, Last::Busy, t(5), None, State::Stopped),
            (false, Last::Busy, t(5), Some(t(9)), State::Stopped),
        ];
        for (finished, last, written, process, want) in rows {
            assert_eq!(state(finished, last, written, process), want, "{finished} {last:?} {written:?} {process:?}");
        }
    }

    fn lines(lines: &[&str]) -> Vec<u8> {
        lines.iter().flat_map(|l| format!("{l}\n").into_bytes()).collect()
    }

    const PROMPT: &str = r#"{"type":"user","isSidechain":true,"uuid":"p","timestamp":"t0","message":{"role":"user","content":"Audit the parser"}}"#;

    #[test]
    fn the_tail_keeps_the_latest_steps_with_their_results() {
        let mut log = vec![PROMPT.to_owned()];
        for n in 1..=7 {
            log.push(format!(r#"{{"type":"assistant","isSidechain":true,"uuid":"a{n}","timestamp":"t{n}","message":{{"stop_reason":"tool_use","content":[{{"type":"tool_use","id":"r{n}","name":"Read","input":{{"file_path":"/w/f{n}.rs"}}}}]}}}}"#));
            log.push(format!(r#"{{"type":"user","isSidechain":true,"uuid":"u{n}","timestamp":"t{n}r","message":{{"content":[{{"type":"tool_result","tool_use_id":"r{n}","content":"ok {n}"}}]}}}}"#));
        }
        log.push(r#"{"type":"assistant","isSidechain":true,"uuid":"b","timestamp":"t8","message":{"stop_reason":"tool_use","content":[{"type":"thinking","thinking":"**Next** the tests."}]}}"#.into());
        log.push(r#"{"type":"assistant","isSidechain":true,"uuid":"c","timestamp":"t9","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"cargo test","description":"Run the tests"}}]}}"#.into());
        log.push(r#"{"type":"api-request","timestamp":"t10"}"#.into());
        let refs: Vec<&str> = log.iter().map(String::as_str).collect();
        let tail = tail_of(&lines(&refs), false);
        let read = |n: u32| Item::Tool { id: format!("r{n}"), at: format!("t{n}"), name: "Read".into(), summary: format!("/w/f{n}.rs"), detail: "".into(), args: vec![] };
        let result = |n: u32| Item::Result { tool: format!("r{n}"), at: format!("t{n}r"), error: false, text: format!("ok {n}"), links: vec![] };
        assert_eq!(
            tail,
            Tail {
                last: Last::Busy,
                updated: "t9".into(),
                steps: vec![
                    read(4),
                    result(4),
                    read(5),
                    result(5),
                    read(6),
                    result(6),
                    read(7),
                    result(7),
                    Item::Thinking { id: "b:0".into(), at: "t8".into(), text: "**Next** the tests.".into() },
                    Item::Tool { id: "b1".into(), at: "t9".into(), name: "Bash".into(), summary: "cargo test".into(), detail: "cargo test".into(), args: vec![] },
                ],
            }
        );
    }

    #[test]
    fn the_tail_drops_the_line_it_started_inside_and_one_still_being_written() {
        let done = r#"{"type":"assistant","isSidechain":true,"uuid":"d","timestamp":"t2","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Report: all good."}]}}"#;
        let mut bytes = b"t\":\"half a line\"}\n".to_vec();
        bytes.extend(lines(&[done]));
        bytes.extend(br#"{"type":"user","isSide"#);
        assert_eq!(tail_of(&bytes, true), Tail { last: Last::Ended, updated: "t2".into(), steps: vec![Item::Text { id: "d:0".into(), at: "t2".into(), text: "Report: all good.".into() }] });
        let stopped = lines(&[PROMPT, r#"{"type":"user","isSidechain":true,"uuid":"s","timestamp":"t3","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#]);
        assert_eq!(tail_of(&stopped, false), Tail { last: Last::Interrupted, updated: "t3".into(), steps: vec![] });
        assert_eq!(tail_of(br#"{"type":"user","message":{"content":"a line too long for the tail"#, true), Tail { last: Last::Busy, updated: "".into(), steps: vec![] });
    }

    #[test]
    fn a_session_folder_lists_its_agents_and_rereads_one_that_grew() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
        write("agent-a1.meta.json", r#"{"agentType":"Explore","description":"Map the parser","toolUseId":"toolu_1","spawnDepth":1,"requestShape":"background","model":"haiku","finished":true}"#);
        write("agent-a1.jsonl", &String::from_utf8(lines(&[PROMPT, r#"{"type":"assistant","isSidechain":true,"uuid":"x","timestamp":"t5","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Mapped."}]}}"#])).unwrap());
        write("agent-a2.meta.json", r#"{"agentType":"general-purpose","description":"Check the tests","toolUseId":"toolu_9","spawnDepth":2,"parentAgentId":"a1"}"#);
        write("agent-a2.jsonl", &String::from_utf8(lines(&[PROMPT, r#"{"type":"assistant","isSidechain":true,"uuid":"y","timestamp":"t6","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"g1","name":"Grep","input":{"pattern":"TODO"}}]}}"#])).unwrap());
        write("notes.txt", "not an agent");
        let mut cache = Cache::default();
        let mapped = Subagent {
            id: "a1".into(),
            tool: "toolu_1".into(),
            parent: None,
            name: "Map the parser".into(),
            kind: "Explore".into(),
            model: Some("haiku".into()),
            state: State::Done,
            updated: "t5".into(),
            steps: vec![Item::Text { id: "x:0".into(), at: "t5".into(), text: "Mapped.".into() }],
        };
        let grep = Item::Tool { id: "g1".into(), at: "t6".into(), name: "Grep".into(), summary: "TODO".into(), detail: "".into(), args: vec![("pattern".into(), "TODO".into())] };
        let checking = Subagent {
            id: "a2".into(),
            tool: "toolu_9".into(),
            parent: Some("a1".into()),
            name: "Check the tests".into(),
            kind: "general-purpose".into(),
            model: None,
            state: State::Running,
            updated: "t6".into(),
            steps: vec![grep.clone()],
        };
        assert_eq!(scan(dir.path(), &mut cache, Some(SystemTime::UNIX_EPOCH)), vec![mapped.clone(), checking.clone()]);
        assert_eq!(scan(dir.path(), &mut cache, None)[1].state, State::Stopped, "an agent of a claude that is gone stopped");

        let mut log = std::fs::OpenOptions::new().append(true).open(dir.path().join("agent-a2.jsonl")).unwrap();
        std::io::Write::write_all(&mut log, &lines(&[r#"{"type":"user","isSidechain":true,"uuid":"z","timestamp":"t7","message":{"content":[{"type":"tool_result","tool_use_id":"g1","content":"No matches found"}]}}"#])).unwrap();
        let grown = scan(dir.path(), &mut cache, Some(SystemTime::UNIX_EPOCH));
        let found = Item::Result { tool: "g1".into(), at: "t7".into(), error: false, text: "No matches found".into(), links: vec![] };
        assert_eq!(grown, vec![mapped, Subagent { updated: "t7".into(), steps: vec![grep, found], ..checking }]);
        assert_eq!(scan(&dir.path().join("missing"), &mut cache, None), vec![]);
    }
}
