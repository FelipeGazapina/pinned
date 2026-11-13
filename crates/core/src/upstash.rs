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
    /// Every box the key lists, newest first.
    #[serde(default)]
    pub boxes: Vec<UsageBox>,
    /// The listed boxes created this UTC month.
    #[serde(default)]
    pub upstash: Totals,
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
    let said = body["error"].as_str().map(str::to_owned).unwrap_or_else(|| format!("request failed with status {status}"));
    let message = match status.as_u16() {
        401 | 403 => format!("Upstash Box refused the key: {said}"),
        _ => format!("Upstash Box answered {}: {said}", status.as_u16()),
    };
    Err(Failure { status: Some(status.as_u16()), message })
}

fn parse_id(body: &Value) -> Result<BoxId, String> {
    body["id"].as_str().filter(|id| !id.is_empty()).map(|id| BoxId(id.to_owned())).ok_or_else(|| format!("Upstash Box sent a box without an id: {body}"))
}

fn parse_status(body: &Value) -> Status {
    match body["status"].as_str() {
        Some("creating") => Status::Creating,
        Some("error" | "deleted") => Status::Failed,
        _ => Status::Ready,
    }
}

fn parse_list(body: &Value) -> Result<Vec<Listed>, String> {
    let boxes = body.as_array().ok_or_else(|| format!("Upstash Box sent a list that isn't one: {body}"))?;
    boxes
        .iter()
        .map(|b| {
            let labels = b["labels"].as_array().map(|ls| ls.iter().filter_map(|l| l.as_str().map(str::to_owned)).collect()).unwrap_or_default();
            Ok(Listed { id: parse_id(b)?, labels })
        })
        .collect()
}

fn parse_all(body: &Value) -> Result<Vec<BoxData>, String> {
    let boxes = body.as_array().ok_or_else(|| format!("Upstash Box sent a list that isn't one: {body}"))?;
    boxes.iter().map(|b| serde_json::from_value(b.clone()).map_err(|e| format!("Upstash Box sent a box pinned cannot read: {e}"))).collect()
}

fn parse_exec(body: &Value) -> Result<Exec, String> {
    let exit_code = body["exit_code"].as_i64().ok_or_else(|| format!("Upstash Box sent an exec result without an exit code: {body}"))?;
    let text = |field: &str| body[field].as_str().unwrap_or_default().to_owned();
    Ok(Exec { exit_code: exit_code as i32, output: text("output"), error: text("error") })
}

/// Checks the key by listing the boxes under `label`.
pub async fn list(key: &str, label: &str) -> Result<Vec<Listed>, String> {
    let path = format!("/v2/box?label={}", urlencode(label));
    parse_list(&call(key, reqwest::Method::GET, &path, None, CALL_WAIT).await?)
}

/// Every box the key lists, whatever its label, for the Usage page.
pub async fn list_all(key: &str) -> Result<Vec<BoxData>, String> {
    parse_all(&call(key, reqwest::Method::GET, "/v2/box", None, LIST_ALL_WAIT).await?)
}

/// Creates the box and waits until it runs.
pub async fn create(key: &str, new: &NewBox) -> Result<BoxId, String> {
    let created = call(key, reqwest::Method::POST, "/v2/box", Some(new.body()), CALL_WAIT).await?;
    let id = parse_id(&created)?;
    let mut status = parse_status(&created);
    let started = tokio::time::Instant::now();
    while status == Status::Creating && started.elapsed() < CREATE_WAIT {
        tokio::time::sleep(CREATE_POLL).await;
        if let Ok(polled) = call(key, reqwest::Method::GET, &format!("/v2/box/{id}"), None, CALL_WAIT).await {
            status = parse_status(&polled);
        }
    }
    match status {
        Status::Ready => Ok(id),
        Status::Creating => Err(format!("Upstash Box was still creating box {id} after {} minutes", CREATE_WAIT.as_secs() / 60)),
        Status::Failed => Err(format!("Upstash Box could not create box {id}")),
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
    parse_exec(&call(key, reqwest::Method::POST, &format!("/v2/box/{id}/exec"), Some(body), EXEC_WAIT).await?)
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

    fn fixture(name: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upstash").join(name);
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
    }

    #[test]
    fn a_listed_box_parses_into_its_id_and_labels() {
        let listed = parse_list(&fixture("list.json")).unwrap();
        assert_eq!(listed, [Listed { id: BoxId("tough-seal-18518".into()), labels: vec!["hub:000000000000".into(), "p:00000000000000ff".into()] }]);
        assert_eq!(parse_list(&json!([{ "id": "b" }])), Ok(vec![Listed { id: BoxId("b".into()), labels: vec![] }]));
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
        let (rows, _) = usage_boxes(&parse_all(&usage_list()).unwrap(), NOVEMBER);
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
        let (_, totals) = usage_boxes(&parse_all(&usage_list()).unwrap(), NOVEMBER);
        assert_eq!(totals.boxes, 3, "the box created a second before the month began is out");
        assert!((totals.cpu_hours - 3.0).abs() < 1e-9);
        assert!((totals.compute_usd - 1.75).abs() < 1e-9);
        assert_eq!(usage_boxes(&[], NOVEMBER).1, Totals::default());
    }

    #[test]
    fn the_month_starts_at_midnight_utc_on_the_first() {
        assert_eq!(month_start(1_794_398_400), NOVEMBER);
        assert_eq!(month_start(NOVEMBER), NOVEMBER);
        assert_eq!(month_start(NOVEMBER - 1), 1_790_812_800, "October 2026 began on the 1st");
    }

    #[test]
    fn a_list_that_is_not_one_is_refused() {
        assert!(parse_all(&json!({ "boxes": [] })).unwrap_err().contains("isn't one"));
        assert!(parse_all(&json!([{ "name": "no id" }])).unwrap_err().contains("cannot read"));
    }

    #[test]
    fn a_created_box_parses_into_its_id_and_waits_while_creating() {
        let created = fixture("create.json");
        assert_eq!(parse_id(&created), Ok(BoxId("tough-seal-18518".into())));
        assert_eq!(parse_status(&created), Status::Creating);
        assert_eq!(parse_status(&fixture("get.json")), Status::Ready);
        assert_eq!(parse_status(&json!({ "id": "b", "status": "error" })), Status::Failed);
    }

    #[test]
    fn an_exec_result_parses_its_exit_code_and_output() {
        let exec = parse_exec(&fixture("exec.json")).unwrap();
        assert_eq!((exec.exit_code, exec.output.lines().next(), exec.error.as_str()), (3, Some("hello"), "err\n"));
        assert!(parse_exec(&json!({ "output": "" })).is_err());
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
