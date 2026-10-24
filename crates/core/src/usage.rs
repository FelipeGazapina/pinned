//! Token use read from Claude Code's transcripts, priced with LiteLLM's public price list, as T3
//! Code's Usage page does.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PRICES_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const PRICES_FRESH: Duration = Duration::from_secs(24 * 3600);
/// A transcript last written this long before the window opens can't hold a line inside it.
const MTIME_SLACK: u64 = 36 * 3600;
/// Bare family names span generations, and `<synthetic>` lines were never billed.
const UNPRICEABLE: [&str; 6] = ["<synthetic>", "synthetic", "opus", "sonnet", "haiku", "fable"];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    Standard,
    Fast,
}

/// One hour's use of one model in one session.
#[derive(Serialize, Debug, PartialEq)]
pub struct Row {
    /// Unix time the hour starts.
    pub hour: u64,
    pub model: String,
    /// Tells the report's sessions apart, which the page counts.
    pub session: u32,
    pub speed: Speed,
    pub input: u64,
    pub cached: u64,
    pub cache_write: u64,
    pub output: u64,
    /// In USD, per token type; `None` for a model the price list doesn't know.
    pub cost: Option<Cost>,
    /// What fast mode added to `cost`.
    pub premium: f64,
    /// What reading `cached` from the cache saved against sending it as input.
    pub savings: f64,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Cost {
    pub input: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub output: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelRate {
    standard: Rates,
    fast: Option<Rates>,
}

pub type RateTable = HashMap<String, ModelRate>;

/// A price the user set for a model, in USD per million tokens; cache prices left out are input's.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

impl Price {
    fn rate(&self) -> ModelRate {
        let m = 1e-6;
        let standard = Rates { input: self.input * m, output: self.output * m, cache_read: self.cache_read.unwrap_or(self.input) * m, cache_write: self.cache_write.unwrap_or(self.input) * m };
        ModelRate { standard, fast: None }
    }
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    timestamp: Option<String>,
    #[serde(rename = "sessionId", default)]
    session: String,
    #[serde(rename = "requestId")]
    request: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Tokens>,
}

#[derive(Deserialize)]
struct Tokens {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    speed: Option<String>,
}

/// Every transcript under `projects` with a line at or after `since`, summed per hour, model,
/// session and speed. Claude Code repeats a message's usage on each of its content blocks and
/// copies messages into resumed and forked sessions, so a message counts once, by id and request.
pub fn read(projects: &Path, since: u64, rates: &RateTable, prices: &HashMap<String, Price>) -> Vec<Row> {
    let mut files = vec![];
    walk(projects, since.saturating_sub(MTIME_SLACK), &mut files);
    files.sort();
    let mut seen = HashSet::new();
    let mut sessions: HashMap<String, u32> = HashMap::new();
    let mut sums: HashMap<(u64, String, u32, Speed), [u64; 4]> = HashMap::new();
    for file in files {
        let Ok(f) = std::fs::File::open(&file) else { continue };
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            if !line.contains("\"usage\"") {
                continue;
            }
            let Ok(line) = serde_json::from_str::<Line>(&line) else { continue };
            let Some(message) = line.message.filter(|_| line.kind == "assistant") else { continue };
            let (Some(tokens), Some(model)) = (message.usage, message.model.filter(|m| !m.is_empty() && m != "<synthetic>")) else { continue };
            let Some(at) = line.timestamp.as_deref().and_then(unix_of).filter(|at| *at >= since) else { continue };
            if (message.id.is_some() || line.request.is_some()) && !seen.insert((message.id, line.request)) {
                continue;
            }
            let next = sessions.len() as u32;
            let session = *sessions.entry(line.session).or_insert(next);
            let speed = if tokens.speed.as_deref() == Some("fast") { Speed::Fast } else { Speed::Standard };
            let sum = sums.entry((at - at % 3600, model, session, speed)).or_default();
            for (total, n) in sum.iter_mut().zip([tokens.input_tokens, tokens.cache_read_input_tokens, tokens.cache_creation_input_tokens, tokens.output_tokens]) {
                *total += n;
            }
        }
    }
    let mut rows: Vec<Row> = sums
        .into_iter()
        .map(|((hour, model, session, speed), [input, cached, cache_write, output])| {
            let mut row = Row { hour, model, session, speed, input, cached, cache_write, output, cost: None, premium: 0.0, savings: 0.0 };
            price(&mut row, rates, prices);
            row
        })
        .collect();
    rows.sort_by(|a, b| (a.hour, &a.model, a.session).cmp(&(b.hour, &b.model, b.session)));
    rows
}

fn walk(dir: &Path, since: u64, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            walk(&path, since, files);
        } else if path.extension().is_some_and(|e| e == "jsonl") && meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).is_some_and(|m| m.as_secs() >= since) {
            files.push(path);
        }
    }
}

fn unix_of(iso: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(iso).ok().and_then(|t| u64::try_from(t.timestamp()).ok())
}

/// A price the user set for the model wins over the list's, even for a name the list can't price.
fn price(row: &mut Row, table: &RateTable, prices: &HashMap<String, Price>) {
    let Some(rate) = prices.get(row.model.trim()).map(Price::rate).or_else(|| lookup(table, &row.model)) else { return };
    let rates = match row.speed {
        Speed::Fast => rate.fast.unwrap_or(rate.standard),
        Speed::Standard => rate.standard,
    };
    let cost = |r: Rates| Cost {
        input: row.input as f64 * r.input,
        cache_read: row.cached as f64 * r.cache_read,
        cache_write: row.cache_write as f64 * r.cache_write,
        output: row.output as f64 * r.output,
    };
    let total = |c: Cost| c.input + c.cache_read + c.cache_write + c.output;
    let listed = cost(rates);
    row.premium = if row.speed == Speed::Standard { 0.0 } else { total(listed) - total(cost(rate.standard)) };
    row.savings = row.cached as f64 * (rates.input - rates.cache_read);
    row.cost = Some(listed);
}

fn lookup(table: &RateTable, model: &str) -> Option<ModelRate> {
    let key = model.trim().to_lowercase();
    let key = key.split('[').next().unwrap_or_default();
    let bare = key.rsplit('/').next().unwrap_or_default();
    if bare.is_empty() || UNPRICEABLE.contains(&bare) {
        return None;
    }
    table.get(key).copied()
}

fn number(entry: &Value, key: &str) -> Option<f64> {
    entry.get(key)?.as_f64().filter(|n| n.is_finite())
}

/// A tier's rates; a cache price the tier leaves out follows the standard tier's ratio to input,
/// or else is input's, since a model priced as if cached input were free would under-report.
fn tier_rates(entry: &Value, suffix: &str, standard: Option<Rates>) -> Option<Rates> {
    let input = number(entry, &format!("input_cost_per_token{suffix}"))?;
    let output = number(entry, &format!("output_cost_per_token{suffix}"))?;
    let cache = |name: &str, pick: fn(&Rates) -> f64| {
        number(entry, &format!("{name}{suffix}")).unwrap_or_else(|| match standard {
            Some(s) if s.input > 0.0 => pick(&s) / s.input * input,
            _ => input,
        })
    };
    Some(Rates { input, output, cache_read: cache("cache_read_input_token_cost", |r| r.cache_read), cache_write: cache("cache_creation_input_token_cost", |r| r.cache_write) })
}

/// LiteLLM's price list as a table, with each `provider/model` also under its bare name when no
/// other entry claims that name at another price.
pub fn parse_rates(document: &Value) -> RateTable {
    let mut table = RateTable::new();
    let Some(entries) = document.as_object() else { return table };
    for (name, entry) in entries {
        let Some(standard) = tier_rates(entry, "", None) else { continue };
        let key = name.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        let multiple = entry.get("provider_specific_entry").and_then(|s| number(s, "fast")).filter(|m| *m > 0.0);
        let fast = match multiple {
            Some(m) => Some(Rates { input: standard.input * m, output: standard.output * m, cache_read: standard.cache_read * m, cache_write: standard.cache_write * m }),
            None => tier_rates(entry, "_priority", Some(standard)),
        };
        table.insert(key, ModelRate { standard, fast });
    }
    let mut aliases: HashMap<String, Option<ModelRate>> = HashMap::new();
    for (key, rate) in &table {
        let bare = key.rsplit('/').next().unwrap_or_default();
        if bare.is_empty() || bare == key || table.contains_key(bare) {
            continue;
        }
        aliases.entry(bare.to_owned()).and_modify(|held| if held.is_some_and(|h| h != *rate) { *held = None }).or_insert(Some(*rate));
    }
    table.extend(aliases.into_iter().filter_map(|(alias, rate)| Some((alias, rate?))));
    table
}

/// The price list, from `cache` while it is a day old at most, else downloaded into it. A failed
/// download falls back to a stale copy, then to no prices, which leaves every model unpriced.
pub fn rates(cache: Option<&Path>) -> RateTable {
    let fresh = |path: &Path| {
        std::fs::metadata(path).ok()?.modified().ok().filter(|m| SystemTime::now().duration_since(*m).unwrap_or_default() < PRICES_FRESH)?;
        std::fs::read(path).ok()
    };
    let body = cache.and_then(fresh).or_else(|| {
        let body = reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)).build().ok()?.get(PRICES_URL).send().ok()?.error_for_status().ok()?.bytes().ok()?.to_vec();
        serde_json::from_slice::<Value>(&body).ok()?;
        if let Some(path) = cache {
            let _ = std::fs::write(path, &body);
        }
        Some(body)
    });
    let body = body.or_else(|| std::fs::read(cache?).ok());
    body.and_then(|b| serde_json::from_slice::<Value>(&b).ok()).map(|doc| parse_rates(&doc)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table() -> RateTable {
        parse_rates(&json!({
            "claude-opus-5-5": { "input_cost_per_token": 5e-6, "output_cost_per_token": 25e-6, "cache_read_input_token_cost": 0.5e-6, "cache_creation_input_token_cost": 6.25e-6,
                "provider_specific_entry": { "fast": 2.0 } },
            "anthropic/claude-haiku-5": { "input_cost_per_token": 1e-6, "output_cost_per_token": 5e-6 },
            "half-priced": { "input_cost_per_token": 1e-6 },
        }))
    }

    fn assistant(id: &str, request: &str, session: &str, at: &str, model: &str, usage: Value) -> String {
        json!({ "type": "assistant", "timestamp": at, "sessionId": session, "requestId": request, "message": { "id": id, "model": model, "role": "assistant", "content": [], "usage": usage } }).to_string()
    }

    #[test]
    fn sums_each_message_once_per_hour_model_and_session() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("-repo");
        std::fs::create_dir_all(project.join("s1/subagents")).unwrap();
        let usage = json!({ "input_tokens": 2, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 100, "output_tokens": 50, "speed": "standard" });
        let lines = [
            assistant("m1", "r1", "s1", "2026-10-24T10:05:00Z", "claude-opus-5-5", usage.clone()),
            // The same message's next content block repeats its usage.
            assistant("m1", "r1", "s1", "2026-10-24T10:05:01Z", "claude-opus-5-5", usage.clone()),
            assistant("m2", "r2", "s1", "2026-10-24T10:55:00Z", "claude-opus-5-5", usage.clone()),
            assistant("m3", "r3", "s1", "2026-10-24T11:00:00Z", "<synthetic>", usage.clone()),
            assistant("m4", "r4", "s1", "2026-10-23T09:00:00Z", "claude-opus-5-5", usage.clone()),
            json!({ "type": "user", "timestamp": "2026-10-24T10:06:00Z", "message": { "role": "user", "content": "usage" } }).to_string(),
        ];
        std::fs::write(project.join("s1.jsonl"), lines.join("\n")).unwrap();
        // A resumed session copies m2 into its own transcript; a subagent's turn counts under its own id.
        let more = [
            assistant("m2", "r2", "s2", "2026-10-24T10:55:00Z", "claude-opus-5-5", usage.clone()),
            assistant("m5", "r5", "s2", "2026-10-24T10:30:00Z", "claude-haiku-5", json!({ "input_tokens": 10, "output_tokens": 20, "speed": "fast" })),
        ];
        std::fs::write(project.join("s2.jsonl"), more.join("\n")).unwrap();
        std::fs::write(project.join("s1/subagents/agent-a.jsonl"), assistant("m6", "r6", "s1", "2026-10-24T10:40:00Z", "claude-opus-5-5", usage.clone())).unwrap();

        let since = unix_of("2026-10-24T00:00:00Z").unwrap();
        let rows = read(dir.path(), since, &table(), &HashMap::new());
        let hour = unix_of("2026-10-24T10:00:00Z").unwrap();
        let summary: Vec<_> = rows.iter().map(|r| (r.hour, r.model.as_str(), r.speed, r.input, r.cached, r.cache_write, r.output)).collect();
        assert_eq!(summary.len(), 2);
        assert!(summary.contains(&(hour, "claude-opus-5-5", Speed::Standard, 6, 3000, 300, 150)));
        assert!(summary.contains(&(hour, "claude-haiku-5", Speed::Fast, 10, 0, 0, 20)));
        let opus = rows.iter().find(|r| r.model == "claude-opus-5-5").unwrap();
        let cost = opus.cost.unwrap();
        assert!((cost.input - 6.0 * 5e-6).abs() < 1e-12);
        assert!((cost.cache_read - 3000.0 * 0.5e-6).abs() < 1e-12);
        assert!((cost.cache_write - 300.0 * 6.25e-6).abs() < 1e-12);
        assert!((cost.output - 150.0 * 25e-6).abs() < 1e-12);
        assert!((opus.savings - 3000.0 * 4.5e-6).abs() < 1e-12);
        let haiku = rows.iter().find(|r| r.model == "claude-haiku-5").unwrap();
        assert_ne!(haiku.session, opus.session);
        assert_eq!(haiku.premium, 0.0, "haiku has no fast tier, so fast is priced as standard");
    }

    #[test]
    fn prices_fast_mode_and_leaves_unknown_models_unpriced() {
        let rates = table();
        let priced = |model: &str, speed| {
            let mut row = Row { hour: 0, model: model.into(), session: 0, speed, input: 1_000_000, cached: 0, cache_write: 0, output: 0, cost: None, premium: 0.0, savings: 0.0 };
            price(&mut row, &rates, &HashMap::from([("opus".to_owned(), Price { input: 2.0, output: 10.0, cache_read: None, cache_write: Some(4.0) })]));
            row
        };
        let fast = priced("claude-opus-5-5[1m]", Speed::Fast);
        assert_eq!(fast.cost.map(|c| c.input), Some(10.0));
        assert_eq!(fast.premium, 5.0);
        let set = priced("opus", Speed::Standard);
        assert_eq!(set.cost.map(|c| c.input), Some(2.0), "the user's price for a bare family name");
        for model in ["half-priced", "claude-unknown-1", "sonnet"] {
            assert_eq!(priced(model, Speed::Standard).cost, None, "{model}");
        }
        assert_eq!(lookup(&rates, "claude-haiku-5").map(|r| r.standard.cache_read), Some(1e-6), "an alias, with cache priced as input");
    }
}
