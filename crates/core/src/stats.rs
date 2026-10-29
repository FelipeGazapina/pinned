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

/// The running totals of a transcript, read up to `offset`.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Tally {
    pub offset: u64,
    /// The last cumulative cost each claude process reported, by the process's `startTime`. A resumed
    /// session counts from zero again, so the thread's cost is their sum.
    pub costs: BTreeMap<u64, f64>,
    pub active_ms: u64,
    pub turns: u32,
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
        for line in bytes[..whole].split(|b| *b == b'\n') {
            let cost = contains(line, br#""cost-state""#);
            if !cost && !contains(line, b"turn_duration") {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
            match (v["type"].as_str(), v["subtype"].as_str()) {
                (Some("cost-state"), _) => {
                    if let (Some(start), Some(usd)) = (v["startTime"].as_u64(), v["totalCostUSD"].as_f64()) {
                        self.costs.insert(start, usd);
                    }
                }
                (Some("system"), Some("turn_duration")) => {
                    if let Some(ms) = v["durationMs"].as_u64() {
                        self.active_ms += ms;
                        self.turns += 1;
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

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
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
    fn stats_leave_out_what_is_unknown_on_the_wire() {
        assert_eq!(serde_json::to_string(&ThreadStats { active_ms: None, cost_usd: Some(0.5) }).unwrap(), r#"{"cost_usd":0.5}"#);
        assert_eq!(serde_json::from_str::<ThreadStats>("{}").unwrap(), ThreadStats::default());
    }
}
