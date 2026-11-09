use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// What a thread's work came to, read from its transcript.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default, schemars::JsonSchema)]
pub struct ThreadStats {
    /// How long the agent worked on the thread's turns, unknown when the entrypoint logs no turn durations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_ms: Option<u64>,
    /// Claude Code's own estimate at API prices, subagents included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// A pull request a conversation opened or worked on.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, schemars::JsonSchema)]
pub struct PrLink {
    pub number: u64,
    pub url: String,
}

/// The running totals of a transcript, read up to `offset`.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Tally {
    pub offset: u64,
    /// The last cumulative cost each claude process reported, by the process's `startTime`. A resumed
    /// session counts from zero again, so the thread's cost is their sum.
    pub costs: BTreeMap<u64, f64>,
    pub active_ms: u64,
    pub turns: u32,
    /// The git branch of the conversation's last message.
    pub branch: Option<String>,
    /// The last pull request the conversation opened or worked on.
    pub pr: Option<PrLink>,
    /// Unix seconds of the conversation's last message.
    pub last_at: Option<u64>,
    /// The first prompt's first line, cut short, which stands in for a title.
    pub prompt_title: Option<String>,
    /// The first prompt went by, so no later line is read for it.
    pub prompted: bool,
}

/// The fields of a transcript line the tally reads. The rest, such as a message's content, is
/// skipped unread.
#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    subtype: Option<String>,
    #[serde(rename = "isSidechain")]
    sidechain: Option<bool>,
    #[serde(rename = "startTime")]
    start: Option<u64>,
    #[serde(rename = "totalCostUSD")]
    cost: Option<f64>,
    #[serde(rename = "durationMs")]
    duration: Option<u64>,
    #[serde(rename = "gitBranch")]
    branch: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "prNumber")]
    pr_number: Option<u64>,
    #[serde(rename = "prUrl")]
    pr_url: Option<String>,
}

impl Tally {
    /// Reads the whole lines `file` gained since the last call, starting over when it got shorter.
    pub fn advance(&mut self, file: &Path) -> std::io::Result<()> {
        let mut f = std::fs::File::open(file)?;
        if self.offset > f.metadata()?.len() {
            *self = Tally::default();
        }
        f.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::with_capacity(1 << 20, f);
        let mut line = vec![];
        loop {
            line.clear();
            reader.read_until(b'\n', &mut line)?;
            if line.last() != Some(&b'\n') {
                return Ok(());
            }
            self.scan(&line);
        }
    }

    /// Adds the whole lines of `bytes`, which follow `offset`, and moves `offset` past them.
    pub fn scan(&mut self, bytes: &[u8]) {
        let whole = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        for raw in bytes[..whole].split(|b| *b == b'\n') {
            let Ok(line) = serde_json::from_slice::<Line>(raw) else { continue };
            match (line.kind.as_deref(), line.subtype.as_deref()) {
                (Some("cost-state"), _) => {
                    if let (Some(start), Some(usd)) = (line.start, line.cost) {
                        self.costs.insert(start, usd);
                    }
                }
                (Some("system"), Some("turn_duration")) => {
                    if let Some(ms) = line.duration {
                        self.active_ms += ms;
                        self.turns += 1;
                    }
                }
                (Some("pr-link"), _) => {
                    if let Some(number) = line.pr_number {
                        self.pr = Some(PrLink { number, url: line.pr_url.unwrap_or_default() });
                    }
                }
                (Some("user" | "assistant"), _) if line.sidechain != Some(true) => {
                    if let Some(branch) = line.branch.filter(|b| !b.is_empty()) {
                        self.branch = Some(branch);
                    }
                    if let Some(at) = line.timestamp.as_deref().and_then(unix_of) {
                        self.last_at = Some(at);
                    }
                    if !self.prompted && line.kind.as_deref() == Some("user") {
                        if let Some(title) = first_prompt(raw) {
                            (self.prompted, self.prompt_title) = (true, title);
                        }
                    }
                }
                _ => {}
            }
        }
        self.offset += whole as u64;
    }

    pub fn stats(&self) -> ThreadStats {
        ThreadStats {
            active_ms: (self.turns > 0).then_some(self.active_ms),
            cost_usd: (!self.costs.is_empty()).then(|| self.costs.values().sum()),
        }
    }
}

/// The title of the prompt on a user line, as the thread shows that prompt: `None` when the line
/// holds no prompt, such as a tool's output, and `Some(None)` for a prompt that makes no title.
fn first_prompt(line: &[u8]) -> Option<Option<String>> {
    let line: Value = serde_json::from_slice(line).ok()?;
    crate::thread::items_of(&line).into_iter().find_map(|item| match item {
        crate::thread::Item::User { text, .. } => Some(crate::hooks::title_from_prompt(&text)),
        _ => None,
    })
}

fn unix_of(timestamp: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(timestamp).ok().and_then(|at| u64::try_from(at.timestamp()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const COST_A1: &str = r#"{"type":"cost-state","sessionId":"s","startTime":100,"totalCostUSD":0.25,"hasUnknownModelCost":false}"#;
    const COST_A2: &str = r#"{"type":"cost-state","sessionId":"s","startTime":100,"totalCostUSD":0.5,"hasUnknownModelCost":false}"#;
    const COST_B1: &str = r#"{"type":"cost-state","sessionId":"s","startTime":200,"totalCostUSD":2.0,"hasUnknownModelCost":false}"#;
    const COST_A3: &str = r#"{"type":"cost-state","sessionId":"s","startTime":100,"totalCostUSD":1.0,"hasUnknownModelCost":false}"#;
    const TURN_60S: &str = r#"{"type":"system","subtype":"turn_duration","durationMs":60000,"uuid":"t1"}"#;
    const TURN_90S: &str = r#"{"type":"system","subtype":"turn_duration","durationMs":90000,"uuid":"t2"}"#;

    fn tally_of(lines: &[&str]) -> Tally {
        let mut t = Tally::default();
        t.scan(format!("{}\n", lines.join("\n")).as_bytes());
        t
    }

    #[test]
    fn cost_sums_the_last_figure_of_each_claude_process() {
        let t = tally_of(&[COST_A1, COST_A2, COST_B1, COST_A3, TURN_60S, TURN_90S]);
        assert_eq!(t.stats(), ThreadStats { active_ms: Some(150_000), cost_usd: Some(3.0) });
    }

    #[test]
    fn a_transcript_with_cost_but_no_turn_durations_has_no_active_time() {
        assert_eq!(tally_of(&[COST_A1]).stats(), ThreadStats { active_ms: None, cost_usd: Some(0.25) });
        assert_eq!(tally_of(&[TURN_60S]).stats(), ThreadStats { active_ms: Some(60_000), cost_usd: None });
    }

    #[test]
    fn unparseable_lines_and_mentions_in_messages_are_skipped() {
        let quoted = r#"{"type":"user","message":{"content":"a line with \"cost-state\" and turn_duration in it"}}"#;
        let t = tally_of(&[r#"{"type":"cost-state","startTime":1,"#, quoted, "turn_duration", COST_A1]);
        assert_eq!(t.stats(), ThreadStats { active_ms: None, cost_usd: Some(0.25) });
    }

    #[test]
    fn a_partial_last_line_waits_for_its_newline() {
        let mut t = Tally::default();
        t.scan(format!("{COST_A1}\n{}", &COST_B1[..20]).as_bytes());
        assert_eq!(t.offset, COST_A1.len() as u64 + 1);
        assert_eq!(t.stats(), ThreadStats { active_ms: None, cost_usd: Some(0.25) });
    }

    #[test]
    fn advance_reads_only_what_the_file_gained() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{COST_A1}\n{TURN_60S}\n{}", &COST_B1[..30]).unwrap();
        let mut t = Tally::default();
        t.advance(file.path()).unwrap();
        assert_eq!(t.stats(), ThreadStats { active_ms: Some(60_000), cost_usd: Some(0.25) });

        writeln!(file, "{}", &COST_B1[30..]).unwrap();
        writeln!(file, "{TURN_90S}").unwrap();
        t.advance(file.path()).unwrap();
        assert_eq!(t.stats(), ThreadStats { active_ms: Some(150_000), cost_usd: Some(2.25) });
        t.advance(file.path()).unwrap();
        assert_eq!(t.stats(), ThreadStats { active_ms: Some(150_000), cost_usd: Some(2.25) }, "nothing new adds nothing");
    }

    #[test]
    fn a_file_replaced_by_a_shorter_one_starts_over() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), format!("{COST_A1}\n{COST_B1}\n{TURN_60S}\n")).unwrap();
        let mut t = Tally::default();
        t.advance(file.path()).unwrap();
        assert_eq!(t.stats(), ThreadStats { active_ms: Some(60_000), cost_usd: Some(2.25) });

        std::fs::write(file.path(), format!("{COST_A2}\n")).unwrap();
        t.advance(file.path()).unwrap();
        assert_eq!(t.stats(), ThreadStats { active_ms: None, cost_usd: Some(0.5) });
    }

    #[test]
    fn the_last_message_names_the_branch_and_time_and_the_last_pr_link_the_pr() {
        let user = r#"{"parentUuid":null,"isSidechain":false,"cwd":"/w/app","sessionId":"s","gitBranch":"main","type":"user","message":{"role":"user","content":"ship \"gitBranch\":\"fake\""},"uuid":"u1","timestamp":"2026-11-09T12:00:00.000Z"}"#;
        let pr = r#"{"type":"pr-link","sessionId":"s","prNumber":92,"prUrl":"https://github.com/o/app/pull/92","prRepository":"o/app","timestamp":"2026-11-09T12:05:00.000Z"}"#;
        let side = r#"{"isSidechain":true,"gitBranch":"side","type":"assistant","message":{},"timestamp":"2026-11-09T13:00:00.000Z"}"#;
        let reply = r#"{"isSidechain":false,"gitBranch":"felipe/fix-and-ops","type":"assistant","message":{"content":[{"type":"text","text":"Done."}]},"uuid":"a1","timestamp":"2026-11-09T12:10:00.000Z"}"#;
        let tool_output = r#"{"isSidechain":false,"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]},"timestamp":"2026-11-09T11:59:00.000Z"}"#;
        let t = tally_of(&[tool_output, user, pr, side, reply, COST_A1]);
        assert_eq!(t.prompt_title.as_deref(), Some("ship \"gitBranch\":\"fake\""), "the first prompt, not a tool's output");
        assert_eq!(t.branch.as_deref(), Some("felipe/fix-and-ops"));
        assert_eq!(t.pr, Some(PrLink { number: 92, url: "https://github.com/o/app/pull/92".into() }));
        assert_eq!(t.last_at, Some(1794226200));
        assert_eq!(t.stats(), ThreadStats { active_ms: None, cost_usd: Some(0.25) });
    }

    #[test]
    fn stats_leave_out_what_is_unknown_on_the_wire() {
        assert_eq!(serde_json::to_string(&ThreadStats { active_ms: None, cost_usd: Some(0.5) }).unwrap(), r#"{"cost_usd":0.5}"#);
        assert_eq!(serde_json::from_str::<ThreadStats>("{}").unwrap(), ThreadStats::default());
    }
}
