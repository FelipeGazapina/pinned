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
/// Rows are this long, so every time zone's hours and days, quarter-hour offsets included, are whole rows.
const SLICE: u64 = 15 * 60;
/// After a failed download of the price list, the next waits this long, so an offline machine doesn't wait out the timeout on every call.
const RETRY_AFTER: Duration = Duration::from_secs(60);
static FAILED_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
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

/// A quarter hour's use of one model in one session.
#[derive(Serialize, Debug, PartialEq)]
pub struct Row {
    /// Unix time the quarter hour starts.
    pub at: u64,
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

/// Every transcript under each of `projects` with a line at or after `since`, summed per quarter hour,
/// model, session and speed, a mapped model counted as the one `book` maps it to. Claude Code
/// repeats a message's usage on each of its content blocks and copies messages into resumed and
/// forked sessions, so a message counts once, by id and request.
pub fn read(projects: &[PathBuf], since: u64, rates: &RateTable, book: &PriceBook) -> Vec<Row> {
    let targets = book.targets();
    let mut files = vec![];
    for dir in projects {
        walk(dir, since.saturating_sub(MTIME_SLACK), &mut files);
    }
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
            let model = targets.get(model.as_str()).map_or(model, |&target| target.to_owned());
            let sum = sums.entry((at - at % SLICE, model, session, speed)).or_default();
            for (total, n) in sum.iter_mut().zip([tokens.input_tokens, tokens.cache_read_input_tokens, tokens.cache_creation_input_tokens, tokens.output_tokens]) {
                *total += n;
            }
        }
    }
    let mut rows: Vec<Row> = sums
        .into_iter()
        .map(|((at, model, session, speed), [input, cached, cache_write, output])| {
            let mut row = Row { at, model, session, speed, input, cached, cache_write, output, cost: None, premium: 0.0, savings: 0.0 };
            price(&mut row, rates, &book.prices);
            row
        })
        .collect();
    rows.sort_by(|a, b| (a.at, &a.model, a.session).cmp(&(b.at, &b.model, b.session)));
    rows
}

/// The user's prices and model mappings on a machine, as T3 Code keeps them in its server settings.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PriceBook {
    #[serde(default)]
    pub prices: HashMap<String, Price>,
    /// A model whose usage counts as another's.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

impl PriceBook {
    pub fn load(path: Option<&Path>) -> PriceBook {
        path.and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Sets each model's price and mapping, or clears it where the change is `None`.
    pub fn apply(&mut self, prices: HashMap<String, Option<Price>>, aliases: HashMap<String, Option<String>>) -> Result<(), String> {
        for (model, price) in prices {
            let model = model.trim().to_owned();
            if model.is_empty() {
                return Err("a price needs a model ID".into());
            }
            match price {
                Some(p) if [Some(p.input), Some(p.output), p.cache_read, p.cache_write].into_iter().flatten().any(|n| !n.is_finite() || n < 0.0) => {
                    return Err(format!("{model}: prices are numbers of 0 or more"));
                }
                Some(p) => self.prices.insert(model, p),
                None => self.prices.remove(&model),
            };
        }
        for (model, alias) in aliases {
            let model = model.trim().to_owned();
            match alias.map(|a| a.trim().to_owned()) {
                Some(a) if a.is_empty() || a == model => return Err(format!("map {model} to a different model")),
                Some(a) => self.aliases.insert(model, a),
                None => self.aliases.remove(&model),
            };
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, serde_json::to_vec_pretty(self).unwrap_or_default()).map_err(|e| format!("couldn't save the prices: {e}"))
    }

    /// Each mapped model's final model, following chains; a mapping that loops counts nowhere,
    /// as T3 Code's `resolveModelAliases` does.
    fn targets(&self) -> HashMap<&str, &str> {
        let mut resolved = HashMap::new();
        for model in self.aliases.keys() {
            let mut seen = HashSet::from([model.as_str()]);
            let mut target = self.aliases[model].as_str();
            while let Some(next) = self.aliases.get(target).filter(|_| !seen.contains(target)) {
                seen.insert(target);
                target = next;
            }
            if !self.aliases.contains_key(target) {
                resolved.insert(model.as_str(), target);
            }
        }
        resolved
    }
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
        if FAILED_AT.lock().unwrap().is_some_and(|at| at.elapsed() < RETRY_AFTER) {
            return None;
        }
        let body = download();
        *FAILED_AT.lock().unwrap() = body.is_none().then(std::time::Instant::now);
        let body = body?;
        if let Some(path) = cache {
            let _ = std::fs::write(path, &body);
        }
        Some(body)
    });
    let body = body.or_else(|| std::fs::read(cache?).ok());
    body.and_then(|b| serde_json::from_slice::<Value>(&b).ok()).map(|doc| parse_rates(&doc)).unwrap_or_default()
}

fn download() -> Option<Vec<u8>> {
    let body = reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)).build().ok()?.get(PRICES_URL).send().ok()?.error_for_status().ok()?.bytes().ok()?.to_vec();
    serde_json::from_slice::<Value>(&body).ok()?;
    Some(body)
}

/// A plan's rate-limit window, as T3 Code's `ServerProviderUsageWindow`.
#[derive(Serialize, Debug, PartialEq)]
pub struct PlanWindow {
    pub id: String,
    /// `session`, `weekly` or `monthly`.
    pub kind: &'static str,
    pub label: String,
    /// Percent of the window used, 0 to 100.
    pub used: f64,
    pub resets_at: Option<u64>,
    /// How long the window runs, for its pace; a spend budget has none.
    pub minutes: Option<u64>,
}

/// The account claude is signed in with and its plan's windows, from claude's own `/usage` check.
#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Limits {
    pub email: Option<String>,
    pub plan: Option<String>,
    pub windows: Vec<PlanWindow>,
    /// Why there are no windows, as T3 Code words it.
    pub unavailable: Option<String>,
}

const LIMITS_TIMEOUT: Duration = Duration::from_secs(25);

/// Asks a `claude` that never gets a prompt for its account and plan limits over the SDK's control
/// protocol, as T3 Code's capability probe does, without hooks, MCP servers or a saved session.
pub fn limits(claude: &Path, home: &Path, path: Option<&str>) -> Result<Limits, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut command = Command::new(claude);
    command
        .args(["-p", "--output-format", "stream-json", "--verbose", "--input-format", "stream-json", "--no-session-persistence", "--strict-mcp-config", "--tools", ""])
        .args(["--settings", r#"{"disableAllHooks":true}"#])
        .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false")
        .env("CLAUDE_CODE_AUTO_CONNECT_IDE", "0")
        .env("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1")
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in crate::machine::INHERITED_SESSION_ENV {
        command.env_remove(key);
    }
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let mut child = command.spawn().map_err(|e| format!("couldn't run claude: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v["type"] == "control_response" && tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    let deadline = std::time::Instant::now() + LIMITS_TIMEOUT;
    let mut ask = |id: &str, request: Value| -> Result<Value, String> {
        let line = serde_json::json!({ "type": "control_request", "request_id": id, "request": request });
        writeln!(stdin, "{line}").map_err(|e| format!("claude closed: {e}"))?;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let answer = rx.recv_timeout(left).map_err(|_| "claude didn't answer in time".to_owned())?;
            let response = &answer["response"];
            if response["request_id"] != id {
                continue;
            }
            if response["subtype"] == "error" {
                return Err(format!("claude: {}", response["error"].as_str().unwrap_or("error")));
            }
            return Ok(response["response"].clone());
        }
    };
    let answers = ask("init", serde_json::json!({ "subtype": "initialize" })).and_then(|init| Ok((init, ask("usage", serde_json::json!({ "subtype": "get_usage", "skip_behaviors": true }))?)));
    let _ = child.kill();
    let _ = child.wait();
    let (init, usage) = answers?;
    Ok(limits_of(&init, &usage))
}

/// T3 Code's `claudeUsageResponseToLimits`: the session and weekly windows, the per-model weekly
/// ones, and the monthly spend budget, with the plan named as its `formatClaudeSubscriptionAuthLabel` does.
fn limits_of(init: &Value, usage: &Value) -> Limits {
    let account = &init["account"];
    let text = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let api_key = account["tokenSource"].as_str().or(account["apiKeySource"].as_str()).is_some_and(|s| {
        let s = s.to_lowercase().replace([' ', '_', '-'], "");
        s == "apikey" || s == "anthropicapikey" || s == "anthropicauthtoken"
    });
    let subscription = text(&account["subscriptionType"]).or_else(|| text(&usage["subscription_type"]));
    let plan = if api_key { Some("Claude API Key".to_owned()) } else { subscription.as_deref().map(plan_label) };
    let mut limits = Limits { email: text(&account["email"]), plan, ..Limits::default() };
    let rates = &usage["rate_limits"];
    if usage["rate_limits_available"] != true || !rates.is_object() {
        let available = usage["rate_limits_available"] == true;
        limits.unavailable = Some(if available { "Could not read limits." } else { "This account has no subscription limits." }.to_owned());
        return limits;
    }
    let clamp = |n: f64| if n.is_finite() { n.clamp(0.0, 100.0) } else { 0.0 };
    let resets = |v: &Value| v.as_str().and_then(unix_of);
    for (id, kind, label, minutes) in [("five_hour", "session", "Session", 300), ("seven_day", "weekly", "Weekly", 10080)] {
        if let Some(used) = rates[id]["utilization"].as_f64() {
            limits.windows.push(PlanWindow { id: id.into(), kind, label: label.into(), used: clamp(used), resets_at: resets(&rates[id]["resets_at"]), minutes: Some(minutes) });
        }
    }
    for scoped in rates["model_scoped"].as_array().into_iter().flatten() {
        let (Some(name), Some(used)) = (scoped["display_name"].as_str(), scoped["utilization"].as_f64()) else { continue };
        let id = format!("seven_day_{}", name.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("_"));
        limits.windows.push(PlanWindow { id, kind: "weekly", label: format!("Weekly · {name}"), used: clamp(used), resets_at: resets(&scoped["resets_at"]), minutes: Some(10080) });
    }
    if let Some(used) = spend_percent(rates) {
        limits.windows.push(PlanWindow { id: "monthly_spend".into(), kind: "monthly", label: "Monthly spend".into(), used: clamp(used), resets_at: None, minutes: None });
    }
    if limits.windows.is_empty() {
        limits.unavailable = Some("No limits reported.".into());
    }
    limits
}

fn spend_percent(rates: &Value) -> Option<f64> {
    let ratio = |used: Option<f64>, limit: Option<f64>| match (used, limit) {
        (Some(u), Some(l)) if u >= 0.0 && l > 0.0 => Some(u / l * 100.0),
        _ => None,
    };
    let spend = &rates["spend"];
    let currency = |m: &Value| m["currency"].as_str().map(str::trim).filter(|c| !c.is_empty()).map(str::to_owned);
    if spend["enabled"] == true && currency(&spend["used"]).is_some() && currency(&spend["used"]) == currency(&spend["limit"]) && spend["used"]["exponent"] == spend["limit"]["exponent"] {
        if let Some(p) = ratio(spend["used"]["amount_minor"].as_f64(), spend["limit"]["amount_minor"].as_f64()) {
            return Some(p);
        }
    }
    let extra = &rates["extra_usage"];
    if extra["is_enabled"] != true {
        return None;
    }
    ratio(extra["used_credits"].as_f64(), extra["monthly_limit"].as_f64()).or(extra["utilization"].as_f64()).filter(|p| p.is_finite() && *p >= 0.0)
}

fn plan_label(subscription: &str) -> String {
    let normalized = subscription.to_lowercase().replace([' ', '_', '-'], "");
    let known = match normalized.as_str() {
        "claudemaxsubscription" | "max" | "maxplan" => Some("Max"),
        "claudemax5xsubscription" | "max5" => Some("Max 5x"),
        "claudemax20xsubscription" | "max20" => Some("Max 20x"),
        "claudeenterprisesubscription" | "enterprise" => Some("Enterprise"),
        "claudeteamsubscription" | "team" => Some("Team"),
        "claudeprosubscription" | "pro" => Some("Pro"),
        "claudefreesubscription" | "free" => Some("Free"),
        _ => None,
    };
    let label = known.map(str::to_owned).unwrap_or_else(|| {
        subscription.split([' ', '_', '-']).filter(|w| !w.is_empty()).map(|w| w[..1].to_uppercase() + &w[1..].to_lowercase()).collect::<Vec<_>>().join(" ")
    });
    let normalized = label.to_lowercase().replace([' ', '_', '-'], "");
    match (normalized.starts_with("claude"), normalized.ends_with("subscription")) {
        (true, true) => label,
        (true, false) => format!("{label} Subscription"),
        (false, true) => format!("Claude {label}"),
        (false, false) => format!("Claude {label} Subscription"),
    }
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
        let rows = read(&[dir.path().to_owned()], since, &table(), &PriceBook::default());
        let quarters: Vec<_> = rows.iter().map(|r| (r.at, r.model.as_str())).collect();
        let at = |iso| unix_of(iso).unwrap();
        assert_eq!(
            quarters,
            [(at("2026-10-24T10:00:00Z"), "claude-opus-5-5"), (at("2026-10-24T10:30:00Z"), "claude-haiku-5"), (at("2026-10-24T10:30:00Z"), "claude-opus-5-5"), (at("2026-10-24T10:45:00Z"), "claude-opus-5-5")]
        );
        let sum = |model: &str, f: fn(&Row) -> f64| rows.iter().filter(|r| r.model == model).map(f).sum::<f64>();
        let opus_tokens = [sum("claude-opus-5-5", |r| r.input as f64), sum("claude-opus-5-5", |r| r.cached as f64), sum("claude-opus-5-5", |r| r.cache_write as f64), sum("claude-opus-5-5", |r| r.output as f64)];
        assert_eq!(opus_tokens, [6.0, 3000.0, 300.0, 150.0], "m1 once, m2 once though resumed into s2, and the subagent's m6");
        let haiku = rows.iter().find(|r| r.model == "claude-haiku-5").unwrap();
        assert_eq!((haiku.speed, haiku.input, haiku.output), (Speed::Fast, 10, 20));
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert!(close(sum("claude-opus-5-5", |r| r.cost.unwrap().input), 6.0 * 5e-6));
        assert!(close(sum("claude-opus-5-5", |r| r.cost.unwrap().cache_read), 3000.0 * 0.5e-6));
        assert!(close(sum("claude-opus-5-5", |r| r.cost.unwrap().cache_write), 300.0 * 6.25e-6));
        assert!(close(sum("claude-opus-5-5", |r| r.cost.unwrap().output), 150.0 * 25e-6));
        assert!(close(sum("claude-opus-5-5", |r| r.savings), 3000.0 * 4.5e-6));
        let opus = rows.iter().find(|r| r.model == "claude-opus-5-5").unwrap();
        assert_ne!(haiku.session, opus.session);
        assert_eq!(haiku.premium, 0.0, "haiku has no fast tier, so fast is priced as standard");
    }

    #[test]
    fn prices_fast_mode_and_leaves_unknown_models_unpriced() {
        let rates = table();
        let priced = |model: &str, speed| {
            let mut row = Row { at: 0, model: model.into(), session: 0, speed, input: 1_000_000, cached: 0, cache_write: 0, output: 0, cost: None, premium: 0.0, savings: 0.0 };
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
    #[test]
    fn a_mapped_model_counts_and_prices_as_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let usage = json!({ "input_tokens": 1_000_000, "output_tokens": 0 });
        let lines = [
            assistant("a", "ra", "s", "2026-10-24T10:05:00Z", "my-opus", usage.clone()),
            assistant("b", "rb", "s", "2026-10-24T10:06:00Z", "claude-opus-5-5", usage.clone()),
            assistant("c", "rc", "s", "2026-10-24T10:07:00Z", "loop-a", usage.clone()),
        ];
        std::fs::write(dir.path().join("s.jsonl"), lines.join("\n")).unwrap();
        let mut book = PriceBook::default();
        let aliases = HashMap::from([("my-opus".to_owned(), Some("alias-mid".to_owned())), ("alias-mid".to_owned(), Some("claude-opus-5-5".to_owned())), ("loop-a".to_owned(), Some("loop-b".to_owned())), ("loop-b".to_owned(), Some("loop-a".to_owned()))]);
        book.apply(HashMap::new(), aliases).unwrap();
        let rows = read(&[dir.path().to_owned()], 0, &table(), &book);
        let models: Vec<_> = rows.iter().map(|r| (r.model.as_str(), r.input, r.cost.map(|c| c.input))).collect();
        assert_eq!(models, [("claude-opus-5-5", 2_000_000, Some(10.0)), ("loop-a", 1_000_000, None)], "a chain resolves, a loop stays as it was");
        assert!(book.apply(HashMap::new(), HashMap::from([("x".to_owned(), Some(" x ".to_owned()))])).is_err());
        assert!(book.apply(HashMap::from([("x".to_owned(), Some(Price { input: -1.0, output: 0.0, cache_read: None, cache_write: None }))]), HashMap::new()).is_err());
        book.apply(HashMap::from([("y".to_owned(), Some(Price { input: 1.0, output: 2.0, cache_read: None, cache_write: None }))]), HashMap::from([("my-opus".to_owned(), None)])).unwrap();
        assert_eq!((book.prices.len(), book.aliases.contains_key("my-opus")), (1, false));
        let file = dir.path().join("prices.json");
        book.save(&file).unwrap();
        assert_eq!(PriceBook::load(Some(&file)), book);
    }

    #[test]
    fn limits_read_claudes_usage_answer_as_t3_code_does() {
        let init = json!({ "account": { "email": "me@example.com", "subscriptionType": "max", "tokenSource": "claude.ai" } });
        let usage = json!({ "rate_limits_available": true, "rate_limits": {
            "five_hour": { "utilization": 23.5, "resets_at": "2026-10-24T12:00:00Z" },
            "seven_day": { "utilization": 140, "resets_at": "2026-10-30T00:00:00Z" },
            "seven_day_opus": { "utilization": 10, "resets_at": null },
            "model_scoped": [{ "display_name": "Fable 5", "utilization": 40, "resets_at": "2026-10-30T00:00:00Z" }, { "display_name": "Unknown", "utilization": null, "resets_at": null }],
            "extra_usage": { "is_enabled": true, "monthly_limit": 5000, "used_credits": 1250, "utilization": 99 },
        } });
        let limits = limits_of(&init, &usage);
        assert_eq!((limits.email.as_deref(), limits.plan.as_deref(), limits.unavailable), (Some("me@example.com"), Some("Claude Max Subscription"), None));
        let windows: Vec<_> = limits.windows.iter().map(|w| (w.id.as_str(), w.kind, w.label.as_str(), w.used, w.minutes)).collect();
        assert_eq!(windows, [
            ("five_hour", "session", "Session", 23.5, Some(300)),
            ("seven_day", "weekly", "Weekly", 100.0, Some(10080)),
            ("seven_day_fable_5", "weekly", "Weekly · Fable 5", 40.0, Some(10080)),
            ("monthly_spend", "monthly", "Monthly spend", 25.0, None),
        ]);
        assert_eq!(limits.windows[0].resets_at, unix_of("2026-10-24T12:00:00Z"));
        let api = limits_of(&json!({ "account": { "tokenSource": "ANTHROPIC_API_KEY" } }), &json!({ "rate_limits_available": false, "rate_limits": null }));
        assert_eq!((api.plan.as_deref(), api.unavailable.as_deref()), (Some("Claude API Key"), Some("This account has no subscription limits.")));
        assert_eq!(plan_label("claude_team_subscription"), "Claude Team Subscription");
        assert_eq!(plan_label("business"), "Claude Business Subscription");
    }
}
