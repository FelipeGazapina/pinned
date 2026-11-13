//! Upstash Box, the cloud sandboxes a workflow can run each session in. The Box REST API's JSON is
//! parsed here and never leaves the module. Endpoints and fields follow `@upstash/box`'s client.

use crate::connectors::BoxSize;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const API: &str = "https://us-east-1.box.upstash.com";
/// Overrides `API`, as the Box SDK and CLI read it.
pub const BASE_URL_ENV: &str = "UPSTASH_BOX_BASE_URL";
pub const KEY_PREFIX: &str = "box_";
/// What `claude setup-token` prints.
pub const CLAUDE_TOKEN_PREFIX: &str = "sk-ant-oat01-";
const CALL_WAIT: Duration = Duration::from_secs(60);
const LIST_ALL_WAIT: Duration = Duration::from_secs(15);
/// The bootstrap downloads pinned-server and maybe gh before it answers.
const EXEC_WAIT: Duration = Duration::from_secs(4 * 60);
const CREATE_POLL: Duration = Duration::from_secs(2);
const CREATE_WAIT: Duration = Duration::from_secs(5 * 60);

/// What the Connectors card shows of the keys pinned keeps.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Account {
    /// The Box key's last characters, as `…a1b2`.
    pub key_hint: String,
    pub claude_hint: String,
}

/// `token` trimmed, once it reads as a `claude setup-token` token.
pub fn claude_token(token: &str) -> Result<String, String> {
    let token = token.trim();
    if !token.starts_with(CLAUDE_TOKEN_PREFIX) {
        return Err(format!("a Claude token from `claude setup-token` starts with {CLAUDE_TOKEN_PREFIX}"));
    }
    Ok(token.to_owned())
}

/// What the hub creates and boots boxes with. Never serialized.
#[derive(Clone)]
pub struct Keys {
    pub api_key: String,
    pub claude_token: String,
}

impl Keys {
    /// Both keys trimmed, once their form is right: a Box key, not the account's Management key,
    /// and a `claude setup-token` token. Only Upstash can tell whether the Box key works.
    pub fn parse(api_key: &str, claude_token: &str) -> Result<Keys, String> {
        let (api_key, claude_token) = (api_key.trim(), claude_token.trim());
        if !api_key.starts_with(KEY_PREFIX) {
            return Err(format!("an Upstash Box key starts with {KEY_PREFIX}: create one in the Box tab of console.upstash.com"));
        }
        Ok(Keys { api_key: api_key.to_owned(), claude_token: self::claude_token(claude_token)? })
    }

    pub fn account(&self) -> Account {
        Account { key_hint: hint(&self.api_key), claude_hint: hint(&self.claude_token) }
    }
}

pub(crate) fn hint(secret: &str) -> String {
    let tail: String = secret.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("…{tail}")
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BoxId(pub String);

impl std::fmt::Display for BoxId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A box to create. It always keeps running while idle and has Chromium, and it never gets
/// `env_vars`: Upstash keeps those in the box's metadata, so secrets go in a file instead.
#[derive(Clone, Debug, PartialEq)]
pub struct NewBox {
    pub name: String,
    pub labels: Vec<String>,
    pub size: BoxSize,
    /// Runs on every container start, after create and on each resume.
    pub init_command: String,
}

impl NewBox {
    fn body(&self) -> Value {
        json!({
            "name": self.name,
            "labels": self.labels,
            "size": self.size,
            "keep_alive": true,
            "init_command": self.init_command,
            "runtime": "node",
            "browser": true,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub id: BoxId,
    pub labels: Vec<String>,
}

/// One box as `GET /v2/box` lists it, with only what the Usage page reads. Every field but the id
/// may be missing; the timestamps are seconds or milliseconds, which `unix_secs` tells apart.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BoxData {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: f64,
    #[serde(default)]
    pub last_activity_at: Option<f64>,
    #[serde(default)]
    pub total_cpu_ns: Option<f64>,
    #[serde(default)]
    pub total_compute_cost_usd: Option<f64>,
}

/// The answer of `HubCall::BoxUsage`: the account's Upstash Box usage, and pinned's own count of
/// the box time its delegated tasks used.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct BoxUsage {
    /// Whether the account saved an Upstash connection. Without one everything else is empty.
    #[serde(default)]
    pub connected: bool,
    /// Why Upstash gave no boxes, in plain text. `boxes` is empty then; `pinned` is still filled.
    #[serde(default)]
    pub error: Option<String>,
    /// Every box on the key that this account opened through pinned, newest first.
    #[serde(default)]
    pub boxes: Vec<UsageBox>,
    /// The boxes created this UTC month, over the whole key: the figures cover every box the key
    /// lists, not only those in `boxes`.
    #[serde(default)]
    pub upstash: Totals,
    /// The other boxes on the key, which this account did not open: a count and sums, no names.
    #[serde(default)]
    pub others: OtherBoxes,
    #[serde(default)]
    pub pinned: PinnedCount,
    /// Boxes pinned could not delete yet, with why. Filled also when Upstash is not connected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stuck: Vec<StuckBox>,
}

/// A box whose delete failed, and the last reason, scrubbed of keys and tokens.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct StuckBox {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub why: String,
    /// The row never recorded an Upstash box id and the delete failed before pinned knew whether a
    /// box exists for it: it may or may not be running. Absent when pinned knows the box is there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unconfirmed: bool,
    /// The box's id on Upstash, for a row that recorded it: what to look for in the console.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstash: Option<String>,
    /// For a row with no recorded Upstash id: the label its box carries there, which pinned lists by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct UsageBox {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub status: String,
    /// Unix seconds.
    #[serde(default)]
    pub created: u64,
    /// Unix seconds.
    #[serde(default)]
    pub last_activity: Option<u64>,
    /// `total_cpu_ns` in hours.
    #[serde(default)]
    pub cpu_hours: Option<f64>,
    /// `total_compute_cost_usd`.
    #[serde(default)]
    pub compute_usd: Option<f64>,
    /// Only when pinned opened this box.
    #[serde(default)]
    pub pinned: Option<PinnedBox>,
}

/// What the hub knows of a box it opened.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct PinnedBox {
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub machine: Option<String>,
}

/// Upstash's own figures for the boxes created this UTC month.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct Totals {
    #[serde(default)]
    pub cpu_hours: f64,
    #[serde(default)]
    pub compute_usd: f64,
    #[serde(default)]
    pub boxes: u32,
}

/// The boxes on the account's Upstash key created this UTC month that the account has no box row
/// for, folded into figures, as the key totals count them. That is the account's own boxes pinned
/// ended (the row goes with the box, Upstash may still list it), boxes made outside pinned, and
/// boxes another account opened on a shared key. The count does not tell which, and no name, id
/// or detail of these is kept.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct OtherBoxes {
    #[serde(default)]
    pub boxes: u32,
    #[serde(default)]
    pub cpu_hours: f64,
    #[serde(default)]
    pub compute_usd: f64,
}

/// pinned's count of the box time its delegated tasks ended with this UTC month. It has no price.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct PinnedCount {
    #[serde(default)]
    pub box_hours: f64,
    #[serde(default)]
    pub tasks: u32,
    /// The month's start, in unix seconds.
    #[serde(default)]
    pub since: u64,
}

/// `at` in unix seconds: a value above 1e12 is milliseconds.
fn unix_secs(at: f64) -> u64 {
    if at > 1e12 {
        (at / 1000.0) as u64
    } else {
        at.max(0.0) as u64
    }
}

/// The first second of the UTC month `now` (unix seconds) falls in.
pub fn month_start(now: u64) -> u64 {
    use chrono::{Datelike, TimeZone, Utc};
    let at = Utc.timestamp_opt(now as i64, 0).single().unwrap_or_default();
    Utc.with_ymd_and_hms(at.year(), at.month(), 1, 0, 0, 0).single().map_or(0, |start| start.timestamp().max(0) as u64)
}

/// `boxes` for the Usage page, newest first, and the totals of those created from `since` on.
pub fn usage_boxes(boxes: &[BoxData], since: u64) -> (Vec<UsageBox>, Totals) {
    let mut rows: Vec<UsageBox> = boxes
        .iter()
        .map(|b| UsageBox {
            id: b.id.clone(),
            name: b.name.clone().unwrap_or_default(),
            size: b.size.clone(),
            status: b.status.clone(),
            created: unix_secs(b.created_at),
            last_activity: b.last_activity_at.map(unix_secs),
            cpu_hours: b.total_cpu_ns.map(|ns| ns / 3.6e12),
            compute_usd: b.total_compute_cost_usd,
            pinned: None,
        })
        .collect();
    rows.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.id.cmp(&b.id)));
    let mut totals = Totals::default();
    for row in rows.iter().filter(|row| row.created >= since) {
        totals.cpu_hours += row.cpu_hours.unwrap_or(0.0);
        totals.compute_usd += row.compute_usd.unwrap_or(0.0);
        totals.boxes += 1;
    }
    (rows, totals)
}

/// `rows` that `opened` names by id, and the rest created from `since` on (the month the key
/// totals cover) folded into one count with their sums. A row not in `opened` is dropped whole:
/// its name, id and detail go no further, and an earlier month's is not counted.
pub fn keep_opened(rows: Vec<UsageBox>, opened: &[&str], since: u64) -> (Vec<UsageBox>, OtherBoxes) {
    let mut others = OtherBoxes::default();
    let mut kept = Vec::new();
    for row in rows {
        if opened.contains(&row.id.as_str()) {
            kept.push(row);
        } else if row.created >= since {
            others.boxes += 1;
            others.cpu_hours += row.cpu_hours.unwrap_or(0.0);
            others.compute_usd += row.compute_usd.unwrap_or(0.0);
        }
    }
    (kept, others)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Exec {
    pub exit_code: i32,
    pub output: String,
    pub error: String,
}

#[derive(Debug, PartialEq)]
enum Status {
    Creating,
    Ready,
    Failed,
}

fn base_url() -> String {
    std::env::var(BASE_URL_ENV).ok().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| API.into()).trim_end_matches('/').to_owned()
}

/// A failure the API answered, with its status, or one that never reached it.
struct Failure {
    status: Option<u16>,
    message: String,
}

impl From<Failure> for String {
    fn from(f: Failure) -> String {
        f.message
    }
}

async fn call(key: &str, method: reqwest::Method, path: &str, body: Option<Value>, wait: Duration) -> Result<Value, Failure> {
    let mut request = reqwest::Client::new().request(method, format!("{}{path}", base_url())).header("X-Box-Api-Key", key).timeout(wait);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let unreachable = |e: reqwest::Error| Failure { status: None, message: format!("Upstash Box unreachable: {e}") };
    let response = request.send().await.map_err(unreachable)?;
    let status = response.status();
    let text = response.text().await.map_err(unreachable)?;
    let body: Value = if text.trim().is_empty() { json!({}) } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
    if status.is_success() {
        return Ok(body);
    }
    Err(Failure { status: Some(status.as_u16()), message: refusal(key, status.as_u16(), &body) })
}

/// The most characters of an Upstash response an error repeats.
const ECHO_LIMIT: usize = 300;

/// `text` as an error may carry it: the key replaced with "…" (an empty key replaces nothing) and
/// no more than `ECHO_LIMIT` characters. The key goes first, so cutting never leaves half of it.
/// A body printed as JSON and a serde error quoting with `{:?}` show a key holding `"`, `\` or a
/// control character escaped, so both escaped forms go too.
/// Every error built from what Upstash sent passes here, as it reaches the log and the app.
fn echoed(key: &str, text: &str) -> String {
    let mut scrubbed = text.to_owned();
    for form in [key.to_owned(), unquoted(&serde_json::to_string(key).unwrap_or_default()), unquoted(&format!("{key:?}"))] {
        if !form.is_empty() {
            scrubbed = scrubbed.replace(&form, "…");
        }
    }
    match scrubbed.char_indices().nth(ECHO_LIMIT) {
        Some((end, _)) => format!("{}…", &scrubbed[..end]),
        None => scrubbed,
    }
}

/// `quoted` without its outer quotes.
fn unquoted(quoted: &str) -> String {
    quoted.strip_prefix('"').and_then(|q| q.strip_suffix('"')).unwrap_or_default().to_owned()
}

/// What Upstash's refusal reads as. Upstash's own `error` text could echo the key.
fn refusal(key: &str, status: u16, body: &Value) -> String {
    let said = body["error"].as_str().map(str::to_owned).unwrap_or_else(|| format!("request failed with status {status}"));
    let message = match status {
        401 | 403 => format!("Upstash Box refused the key: {said}"),
        _ => format!("Upstash Box answered {status}: {said}"),
    };
    echoed(key, &message)
}

fn parse_id(key: &str, body: &Value) -> Result<BoxId, String> {
    body["id"].as_str().filter(|id| !id.is_empty()).map(|id| BoxId(id.to_owned())).ok_or_else(|| echoed(key, &format!("Upstash Box sent a box without an id: {body}")))
}

fn parse_status(body: &Value) -> Status {
    match body["status"].as_str() {
        Some("creating") => Status::Creating,
        Some("error" | "deleted") => Status::Failed,
        _ => Status::Ready,
    }
}

fn parse_list(key: &str, body: &Value) -> Result<Vec<Listed>, String> {
    let boxes = body.as_array().ok_or_else(|| echoed(key, &format!("Upstash Box sent a list that isn't one: {body}")))?;
    boxes
        .iter()
        .map(|b| {
            let labels = b["labels"].as_array().map(|ls| ls.iter().filter_map(|l| l.as_str().map(str::to_owned)).collect()).unwrap_or_default();
            Ok(Listed { id: parse_id(key, b)?, labels })
        })
        .collect()
}

fn parse_all(key: &str, body: &Value) -> Result<Vec<BoxData>, String> {
    let boxes = body.as_array().ok_or_else(|| echoed(key, &format!("Upstash Box sent a list that isn't one: {body}")))?;
    boxes.iter().map(|b| serde_json::from_value(b.clone()).map_err(|e| echoed(key, &format!("Upstash Box sent a box pinned cannot read: {e}")))).collect()
}

fn parse_exec(key: &str, body: &Value) -> Result<Exec, String> {
    let exit_code = body["exit_code"].as_i64().ok_or_else(|| echoed(key, &format!("Upstash Box sent an exec result without an exit code: {body}")))?;
    let text = |field: &str| body[field].as_str().unwrap_or_default().to_owned();
    Ok(Exec { exit_code: exit_code as i32, output: text("output"), error: text("error") })
}

/// Checks the key by listing the boxes under `label`.
pub async fn list(key: &str, label: &str) -> Result<Vec<Listed>, String> {
    let path = format!("/v2/box?label={}", urlencode(label));
    parse_list(key, &call(key, reqwest::Method::GET, &path, None, CALL_WAIT).await?)
}

/// Every box the key lists, whatever its label, for the Usage page.
pub async fn list_all(key: &str) -> Result<Vec<BoxData>, String> {
    parse_all(key, &call(key, reqwest::Method::GET, "/v2/box", None, LIST_ALL_WAIT).await?)
}

/// Creates the box and waits until it runs.
pub async fn create(key: &str, new: &NewBox) -> Result<BoxId, String> {
    let created = call(key, reqwest::Method::POST, "/v2/box", Some(new.body()), CALL_WAIT).await?;
    let id = parse_id(key, &created)?;
    let mut status = parse_status(&created);
    let started = tokio::time::Instant::now();
    while status == Status::Creating && started.elapsed() < CREATE_WAIT {
        tokio::time::sleep(CREATE_POLL).await;
        if let Ok(polled) = call(key, reqwest::Method::GET, &format!("/v2/box/{id}"), None, CALL_WAIT).await {
            status = parse_status(&polled);
        }
    }
    finished(key, id, status)
}

/// How `create` ends once it has stopped waiting.
fn finished(key: &str, id: BoxId, status: Status) -> Result<BoxId, String> {
    match status {
        Status::Ready => Ok(id),
        Status::Creating => Err(echoed(key, &format!("Upstash Box was still creating box {id} after {} minutes", CREATE_WAIT.as_secs() / 60))),
        Status::Failed => Err(echoed(key, &format!("Upstash Box could not create box {id}"))),
    }
}

/// Writes `content` to the absolute `path`. The file loses any executable bit.
pub async fn write(key: &str, id: &BoxId, path: &str, content: &str) -> Result<(), String> {
    let body = json!({ "path": path, "content": content });
    call(key, reqwest::Method::POST, &format!("/v2/box/{id}/files/write"), Some(body), CALL_WAIT).await.map(drop).map_err(String::from)
}

/// Runs `script` with `sh -c`.
pub async fn exec(key: &str, id: &BoxId, script: &str) -> Result<Exec, String> {
    let body = json!({ "command": ["sh", "-c", script] });
    parse_exec(key, &call(key, reqwest::Method::POST, &format!("/v2/box/{id}/exec"), Some(body), EXEC_WAIT).await?)
}

/// Succeeds when the box is already gone.
pub async fn delete(key: &str, id: &BoxId) -> Result<(), String> {
    match call(key, reqwest::Method::DELETE, &format!("/v2/box/{id}"), None, CALL_WAIT).await {
        Ok(_) => Ok(()),
        Err(Failure { status: Some(404), .. }) => Ok(()),
        Err(f) => Err(f.message),
    }
}

fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_that_echoes_the_key_does_not_carry_it() {
        let key = "box_fake-key-0000";
        for status in [401u16, 403, 404, 500] {
            let body = json!({ "error": format!("bad key {key} for this account ({key})") });
            let message = refusal(key, status, &body);
            assert!(!message.contains(key), "{status}: {message}");
            assert!(message.contains("bad key … for this account (…)"), "{status}: {message}");
        }
        assert_eq!(refusal(key, 500, &json!({})), "Upstash Box answered 500: request failed with status 500");
    }

    #[test]
    fn a_2xx_body_that_is_not_what_was_expected_does_not_carry_the_key_into_the_error() {
        let key = "box_fake-key-0000";
        let echoing = |field: &str| json!({ field: key });
        let errors = [
            parse_id(key, &echoing("note")).unwrap_err(),
            parse_list(key, &echoing("note")).unwrap_err(),
            parse_list(key, &json!([echoing("note")])).unwrap_err(),
            parse_all(key, &echoing("note")).unwrap_err(),
            parse_all(key, &json!([{ "id": "b", "created_at": key }])).unwrap_err(),
            parse_exec(key, &echoing("note")).unwrap_err(),
        ];
        for message in &errors {
            assert!(!message.contains(key), "{message}");
            assert!(message.contains('…'), "the key is replaced, not dropped: {message}");
        }
        let long = json!({ "note": format!("{key}{}", "x".repeat(10_000)) });
        let message = parse_id(key, &long).unwrap_err();
        assert!(!message.contains(key) && message.chars().count() < 500, "a body is echoed in part: {} chars", message.chars().count());
    }

    #[test]
    fn a_key_with_characters_json_and_debug_escape_does_not_reach_the_error_in_either_form() {
        for key in ["box_fake\"key\\0000", "box_fake\u{1}key\u{7f}0000"] {
            let json_form = serde_json::to_string(key).unwrap();
            let debug_form = format!("{key:?}");
            let forms = [key, &json_form[1..json_form.len() - 1], &debug_form[1..debug_form.len() - 1]];
            let echoing = json!({ "note": key });
            let unreadable = json!([{ "id": "b", "created_at": key }]);
            let errors = [parse_id(key, &echoing).unwrap_err(), parse_all(key, &unreadable).unwrap_err()];
            for message in &errors {
                for form in forms {
                    assert!(!message.contains(form), "{form:?} is in: {message}");
                }
                assert!(message.contains('…'), "the key is replaced, not dropped: {message}");
            }
        }
    }

    #[test]
    fn a_created_box_that_never_runs_does_not_carry_the_key_or_a_long_id_into_the_error() {
        let key = "box_fake-key-0000";
        let id = BoxId(format!("{key}{}", "x".repeat(10_000)));
        assert_eq!(finished(key, BoxId("b".into()), Status::Ready), Ok(BoxId("b".into())));
        for status in [Status::Creating, Status::Failed] {
            let message = finished(key, id.clone(), status).unwrap_err();
            assert!(!message.contains(key), "{message}");
            assert!(message.contains('…') && message.chars().count() < 500, "{} chars: {message}", message.chars().count());
        }
    }

    fn fixture(name: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upstash").join(name);
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
    }

    #[test]
    fn a_listed_box_parses_into_its_id_and_labels() {
        let listed = parse_list("", &fixture("list.json")).unwrap();
        assert_eq!(listed, [Listed { id: BoxId("tough-seal-18518".into()), labels: vec!["hub:000000000000".into(), "p:00000000000000ff".into()] }]);
        assert_eq!(parse_list("", &json!([{ "id": "b" }])), Ok(vec![Listed { id: BoxId("b".into()), labels: vec![] }]));
    }

    const NOVEMBER: u64 = 1_793_491_200;

    fn usage_list() -> Value {
        json!([
            { "id": "a", "name": "web", "labels": ["hub:1"], "size": "large", "status": "running", "runtime": "node", "created_at": 1_793_588_400u64, "updated_at": 1_793_600_000u64,
              "last_activity_at": 1_794_398_400u64, "total_cpu_ns": 7.2e12, "total_compute_cost_usd": 1.5, "total_token_cost_usd": 4.0, "extra": true },
            { "id": "b", "status": "paused", "created_at": 1_793_836_800u64, "updated_at": 1_793_836_801u64 },
            { "id": "c", "name": "ms", "status": "idle", "created_at": 1_793_700_000_000u64, "updated_at": 1_793_700_000_001u64, "last_activity_at": 1_793_700_005_000u64,
              "total_cpu_ns": 3.6e12, "total_compute_cost_usd": 0.25 },
            { "id": "d", "name": "old", "status": "deleted", "created_at": 1_793_491_199u64, "updated_at": 1_793_491_199u64, "total_cpu_ns": 3.6e13, "total_compute_cost_usd": 9.0 }
        ])
    }

    #[test]
    fn listed_boxes_become_usage_rows_newest_first() {
        let (rows, _) = usage_boxes(&parse_all("", &usage_list()).unwrap(), NOVEMBER);
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["b", "c", "a", "d"]);
        let (a, b, c) = (&rows[2], &rows[0], &rows[1]);
        assert_eq!((a.name.as_str(), a.size.as_deref(), a.status.as_str(), a.created, a.last_activity), ("web", Some("large"), "running", 1_793_588_400, Some(1_794_398_400)));
        assert!((a.cpu_hours.unwrap() - 2.0).abs() < 1e-9);
        assert_eq!((a.compute_usd, a.pinned.clone()), (Some(1.5), None));
        assert_eq!(*b, UsageBox { id: "b".into(), name: "".into(), size: None, status: "paused".into(), created: 1_793_836_800, last_activity: None, cpu_hours: None, compute_usd: None, pinned: None });
        assert_eq!((c.created, c.last_activity), (1_793_700_000, Some(1_793_700_005)), "a millisecond stamp reads as seconds");
        assert!((c.cpu_hours.unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn totals_count_only_the_boxes_created_this_month() {
        let (_, totals) = usage_boxes(&parse_all("", &usage_list()).unwrap(), NOVEMBER);
        assert_eq!(totals.boxes, 3, "the box created a second before the month began is out");
        assert!((totals.cpu_hours - 3.0).abs() < 1e-9);
        assert!((totals.compute_usd - 1.75).abs() < 1e-9);
        assert_eq!(usage_boxes(&[], NOVEMBER).1, Totals::default());
    }

    #[test]
    fn keep_opened_lists_the_named_boxes_and_folds_every_other_into_sums() {
        let (rows, _) = usage_boxes(&parse_all("", &usage_list()).unwrap(), NOVEMBER);
        let (kept, others) = keep_opened(rows.clone(), &["a"], NOVEMBER);
        assert_eq!(kept.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(others.boxes, 2, "d was created before the month began, so it is not counted");
        assert!((others.cpu_hours - 1.0).abs() < 1e-9, "b has no figure, c is 1 h");
        assert!((others.compute_usd - 0.25).abs() < 1e-9);
        assert_eq!(keep_opened(rows.clone(), &[], NOVEMBER).0, vec![], "nothing opened lists nothing");
        assert_eq!(keep_opened(rows.clone(), &["a", "b", "c"], NOVEMBER).1, OtherBoxes::default(), "an earlier month's box is not an other box");
        assert_eq!(keep_opened(rows, &["a"], 0).1.boxes, 3, "with no month limit every unlisted box counts");
    }

    #[test]
    fn the_month_starts_at_midnight_utc_on_the_first() {
        assert_eq!(month_start(1_794_398_400), NOVEMBER);
        assert_eq!(month_start(NOVEMBER), NOVEMBER);
        assert_eq!(month_start(NOVEMBER - 1), 1_790_812_800, "October 2026 began on the 1st");
    }

    #[test]
    fn a_list_that_is_not_one_is_refused() {
        assert!(parse_all("", &json!({ "boxes": [] })).unwrap_err().contains("isn't one"));
        assert!(parse_all("", &json!([{ "name": "no id" }])).unwrap_err().contains("cannot read"));
    }

    #[test]
    fn a_created_box_parses_into_its_id_and_waits_while_creating() {
        let created = fixture("create.json");
        assert_eq!(parse_id("", &created), Ok(BoxId("tough-seal-18518".into())));
        assert_eq!(parse_status(&created), Status::Creating);
        assert_eq!(parse_status(&fixture("get.json")), Status::Ready);
        assert_eq!(parse_status(&json!({ "id": "b", "status": "error" })), Status::Failed);
    }

    #[test]
    fn an_exec_result_parses_its_exit_code_and_output() {
        let exec = parse_exec("", &fixture("exec.json")).unwrap();
        assert_eq!((exec.exit_code, exec.output.lines().next(), exec.error.as_str()), (3, Some("hello"), "err\n"));
        assert!(parse_exec("", &json!({ "output": "" })).is_err());
    }

    #[test]
    fn a_new_box_keeps_running_with_a_browser_and_no_env_vars() {
        let new = NewBox { name: "pinned-00c0ffee".into(), labels: vec!["hub:3f9a1c2b7d4e".into(), "p:00c0ffee12345678".into()], size: BoxSize::Medium, init_command: "sh /workspace/home/.pinned/start.sh || true".into() };
        assert_eq!(
            new.body(),
            json!({
                "name": "pinned-00c0ffee", "labels": ["hub:3f9a1c2b7d4e", "p:00c0ffee12345678"], "size": "medium", "keep_alive": true,
                "init_command": "sh /workspace/home/.pinned/start.sh || true", "runtime": "node", "browser": true,
            })
        );
    }

    #[test]
    fn keys_need_a_box_key_and_a_setup_token() {
        assert_eq!(Keys::parse("9b2c5a1e-uuid", "sk-ant-oat01-x").err().unwrap(), "an Upstash Box key starts with box_: create one in the Box tab of console.upstash.com");
        assert_eq!(Keys::parse("box_k", "sk-ant-api03-x").err().unwrap(), "a Claude token from `claude setup-token` starts with sk-ant-oat01-");
        let keys = Keys::parse(" box_abcdA1b2 ", " sk-ant-oat01-zz9f3c\n").unwrap();
        assert_eq!((keys.api_key.as_str(), keys.claude_token.as_str()), ("box_abcdA1b2", "sk-ant-oat01-zz9f3c"));
        assert_eq!(keys.account(), Account { key_hint: "…A1b2".into(), claude_hint: "…9f3c".into() });
    }

    #[test]
    fn a_label_is_escaped_in_the_list_query() {
        assert_eq!(urlencode("hub:3f9a"), "hub%3A3f9a");
    }
}
