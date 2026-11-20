//! OpenCode Go, as an auth mode of the Claude Code provider. The `claude` CLI talks to Go's
//! Anthropic Messages gateway. The model list is fetched live from `GET /inference/go/v1/models`
//! and cached on disk, so a model the gateway adds shows up without a pinned release. The table
//! below is only the fallback when the gateway cannot be reached. An id that is not in any list
//! still works when a caller passes it as `target.model`.

use crate::agent::{LimitWindow, Usage};
use crate::claude_models::CLAUDE_EFFORT;
use crate::delegate::ModelChoice;
use serde_json::Value;
use std::time::{Duration, Instant};

/// Set on a Go session so the statusline can ask Go for usage. Never a login account.
pub const KEY_ENV: &str = "PINNED_OPENCODE_GO_KEY";
pub const BASE_URL: &str = "https://opencode.ai/inference/go/anthropic";
/// Points a Go session at another gateway, a test's or a local proxy's.
pub const BASE_ENV: &str = "PINNED_OPENCODE_GO_BASE";
pub const MODELS_URL: &str = "https://opencode.ai/inference/go/v1/models";
pub const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";
pub const DEFAULT_OPUS_MODEL: &str = "kimi-k3";
pub const DEFAULT_SONNET_MODEL: &str = "minimax-m3";
pub const DEFAULT_HAIKU_MODEL: &str = "claude-haiku-5-5";
pub const SMALL_FAST_MODEL: &str = "claude-haiku-5-5";

/// What a Go session must not inherit from the shell or a parent Claude session. `claude` reads
/// these before it reads the Go env, and an API key or a Bedrock flag would otherwise win.
pub const STRIP_ENV: [&str; 16] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_GATEWAY",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_USE_VERTEX",
];

const USER_AGENT: &str = concat!("pinned/", env!("CARGO_PKG_VERSION"));
const NO_EFFORT: &[&str] = &[];

/// How long a fetched gateway list stays fresh.
pub(crate) const REFRESH_EVERY: Duration = Duration::from_secs(3600);
/// How soon a failed fetch is tried again: a down gateway must not cost one request per account read.
pub(crate) const RETRY_AFTER: Duration = Duration::from_secs(60);

/// Whether the gateway list is due for a fetch: hourly after a success, and a minute after a
/// failure, instead of on every read.
pub(crate) fn refresh_due(last: Option<Instant>, failed: bool, now: Instant) -> bool {
    last.is_none_or(|at| now.saturating_duration_since(at) >= if failed { RETRY_AFTER } else { REFRESH_EVERY })
}

/// The file a machine keeps the last gateway model list in, so the picker lists new models
/// without a pinned release. The gateway remains the source of truth on every refresh.
pub const GO_MODELS_FILE: &str = "opencode-go-models.json";

/// `(id, label, efforts, context, free)`: the last gateway list pinned shipped. It only runs
/// when the gateway cannot be reached and no fresher cache exists.
const MODELS: &[(&str, &str, &[&str], u64, bool)] = &[
    ("deepseek-v4.1-flash", "DeepSeek V4.1 Flash 1017", &["low", "high", "max"], 1_000_000, false),
    ("deepseek-v4-pro", "DeepSeek V4 Pro", &["high", "max"], 1_000_000, false),
    ("glm-5.3", "GLM-5.3", &["low", "high", "max"], 1_000_000, false),
    ("grok-4.7", "Grok 4.7", &["low", "medium", "high", "xhigh"], 500_000, false),
    ("muse-spark-1.2-contributor", "Muse Spark 1.2 Contributor", &["minimal", "low", "medium", "high", "xhigh"], 1_048_576, false),
    ("muse-spark-1.3-contributor", "Muse Spark 1.3 Contributor", &["minimal", "low", "medium", "high", "xhigh"], 1_048_576, false),
    ("glm-5.3-flash", "GLM-5.3-Flash", &["low", "high", "max"], 1_000_000, false),
    ("gpt-5.6-luna", "GPT-5.6 Luna", &["none", "low", "medium", "high", "xhigh", "max"], 1_050_000, false),
    ("gpt-6-luna", "GPT-6 Luna", &["none", "low", "medium", "high", "xhigh", "max"], 1_050_000, false),
    ("claude-haiku-5-5", "Claude Haiku 5.5", &["low", "medium", "high", "xhigh", "max"], 1_000_000, false),
    ("hy3", "Hy3", &["none", "low", "high"], 256_000, false),
    ("hy4-preview", "Hy4 preview", &["none", "high"], 1_024_000, false),
    ("kimi-k2.7-code", "Kimi K2.7 Code", NO_EFFORT, 262_144, false),
    ("kimi-k3", "Kimi K3", &["max"], 1_048_576, false),
    ("mimo-v2.6-flash", "MiMo-V2.6-Flash", NO_EFFORT, 1_048_576, false),
    ("mimo-v2.6-pro", "MiMo-V2.6-Pro", NO_EFFORT, 1_048_576, false),
    ("minimax-m3", "MiniMax-M3", NO_EFFORT, 1_000_000, false),
    ("minimax-m3.1-flash", "MiniMax M3.1 Flash", &["low", "medium", "high", "xhigh", "max"], 1_000_000, false),
    ("longcat-2.0", "LongCat-2.0", NO_EFFORT, 1_000_000, false),
    ("step-5-preview", "Step 5 Preview", &["low", "medium", "high"], 1_000_000, false),
    ("beam-free", "Beam Free", &["low", "medium", "high", "xhigh", "max"], 1_048_576, true),
    ("qwen3.8-max", "Qwen3.8 Max", &["low", "medium", "xhigh"], 1_000_000, false),
    ("qwen3.8-flash", "Qwen3.8 Flash", &["low", "medium", "xhigh"], 1_000_000, false),
    ("qwen3.7-plus", "Qwen3.7 Plus", NO_EFFORT, 1_000_000, false),
    ("kimi-k3.1-pro", "Kimi K3.1 Pro", &["low", "high", "max"], 1_048_576, false),
    ("iceman-contributor-free", "Iceman Contributor Free", NO_EFFORT, 262_144, true),
    ("gemini-4-argon", "Gemini 4 Argon", &["low", "medium", "high"], 1_048_576, false),
    ("hy4", "Hy4", &["none", "high"], 1_048_576, false),
    ("longcat-2.5-free", "LongCat 2.5 Free", NO_EFFORT, 1_000_000, true),
    ("hy4-flash-free", "Hy4 flash Free", &["none", "high"], 1_048_576, true),
    ("qwen4-max", "Qwen4 Max", &["low", "high", "xhigh"], 1_000_000, false),
];

pub fn models() -> Vec<ModelChoice> {
    fallback_models()
        .into_iter()
        .map(|m| ModelChoice {
            id: m.slug.clone(),
            label: m.name.clone(),
            efforts: m.efforts.iter().filter(|e| CLAUDE_EFFORT.contains(&e.as_str())).cloned().collect(),
            cost: m.cost,
            free: m.free,
        })
        .collect()
}

/// The fallback list as picker models, with context windows and free flags, in gateway order. The
/// shipped table lists the gateway's own values, `none` and `minimal` included, so it is filtered
/// the same way the live list is.
pub fn fallback_models() -> Vec<crate::claude_models::ClaudeModel> {
    MODELS
        .iter()
        .map(|(id, label, efforts, context, free)| {
            let free = *free;
            crate::claude_models::ClaudeModel {
                slug: (*id).into(),
                name: (*label).into(),
                menu: None,
                efforts: efforts.iter().filter(|e| CLAUDE_EFFORT.contains(*e)).map(|e| (*e).to_string()).collect(),
                context: *context,
                cost: if free { crate::delegate::Cost::Low } else { crate::delegate::cost_of(id) },
                legacy: false,
                new: false,
                fast: false,
                free,
            }
        })
        .collect()
}

/// What a Go session sets besides `KEY_ENV`, in the order `env` sets them.
pub const SESSION_ENV: [&str; 6] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
];

/// What a Go session adds after pinned has stripped a parent shell's Anthropic variables.
pub fn env(key: &str) -> Vec<(String, String)> {
    let key = key.trim().to_owned();
    let values = [
        gateway_from(std::env::var(BASE_ENV).ok().as_deref()),
        key.clone(),
        DEFAULT_OPUS_MODEL.into(),
        DEFAULT_SONNET_MODEL.into(),
        DEFAULT_HAIKU_MODEL.into(),
        SMALL_FAST_MODEL.into(),
    ];
    std::iter::once((KEY_ENV.to_owned(), key)).chain(SESSION_ENV.iter().map(|name| name.to_string()).zip(values)).collect()
}

/// The gateway model list as picker models, in the order the gateway listed them. A model the
/// gateway stops listing simply drops out, and one it adds shows up, with no pinned release.
/// `free` is true when both token prices are zero. Unknown shapes are skipped.
pub fn parse_models(body: &str) -> Vec<crate::claude_models::ClaudeModel> {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let data = value.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut models = Vec::with_capacity(data.len());
    for item in data {
        let Some(id) = item.get("id").and_then(Value::as_str) else { continue };
        if id.trim().is_empty() {
            continue;
        }
        let name = item.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or(id).to_owned();
        let efforts: Vec<String> = item
            .pointer("/capabilities/reasoning/effort")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).filter(|e| CLAUDE_EFFORT.contains(e)).map(str::to_owned).collect())
            .unwrap_or_default();
        let context = item.pointer("/limit/context").and_then(Value::as_u64).filter(|c| *c > 0).unwrap_or(1_000_000);
        let price = |path: &str| item.pointer(path).and_then(Value::as_f64).unwrap_or(-1.0);
        let free = price("/cost/text/input/price") == 0.0 && price("/cost/text/output/price") == 0.0;
        models.push(crate::claude_models::ClaudeModel {
            slug: id.to_owned(),
            name,
            menu: None,
            efforts,
            context,
            cost: if free { crate::delegate::Cost::Low } else { crate::delegate::cost_of(id) },
            legacy: false,
            new: false,
            fast: false,
            free,
        });
    }
    models
}

/// Fetches the gateway model list. The endpoint is public, so no key is sent.
pub fn fetch_models() -> Result<Vec<crate::claude_models::ClaudeModel>, String> {
    let url = format!("{}/v1/models", models_base());
    let (status, body) = get_public(&url, Duration::from_secs(10))?;
    if !(200..300).contains(&status) {
        return Err(format!("OpenCode Go answered {status}."));
    }
    let models = parse_models(&body);
    if models.is_empty() {
        return Err("OpenCode Go listed no models.".into());
    }
    Ok(models)
}

/// Reads a cached gateway list, when it parses. A cache written before the efforts were filtered
/// still lists `none` and `minimal`, so the read drops them too.
pub fn load_cached(path: &std::path::Path) -> Option<Vec<crate::claude_models::ClaudeModel>> {
    let body = std::fs::read(path).ok()?;
    let mut models: Vec<crate::claude_models::ClaudeModel> = serde_json::from_slice(&body).ok()?;
    for model in &mut models {
        model.efforts.retain(|e| CLAUDE_EFFORT.contains(&e.as_str()));
    }
    (!models.is_empty()).then_some(models)
}

/// The gateway a Go session gets when `BASE_ENV` is `value`.
pub fn gateway_from(value: Option<&str>) -> String {
    value.and_then(allowed_override).unwrap_or_else(|| BASE_URL.to_owned())
}

/// Asks the models endpoint whether `key` is a Go key. `PINNED_OPENCODE_GO_MODELS_BASE` points a
/// test or a proxy at another origin.
pub fn validate(key: &str) -> Result<(), String> {
    validate_at(&models_base(), key)
}

fn models_base() -> String {
    override_url("PINNED_OPENCODE_GO_MODELS_BASE").unwrap_or_else(|| MODELS_URL.trim_end_matches("/v1/models").to_owned())
}

pub fn validate_at(base: &str, key: &str) -> Result<(), String> {
    let base = base.trim().trim_end_matches('/');
    let (status, _) = get(&format!("{base}/v1/models"), key, Duration::from_secs(15))?;
    match status {
        200..=299 => Ok(()),
        401 => Err("OpenCode Go rejected this key.".into()),
        403 => Err("This key's workspace has no OpenCode Go subscription.".into()),
        other => Err(format!("OpenCode Go answered {other}.")),
    }
}

/// The rolling, weekly and monthly allowances. The Console docs say a bearer key is enough, so
/// this sends no workspace header.
pub fn usage(key: &str) -> Result<Usage, String> {
    usage_at(&usage_url(), key)
}

pub fn usage_at(url: &str, key: &str) -> Result<Usage, String> {
    let (status, body) = get(url, key, Duration::from_secs(5))?;
    if !(200..300).contains(&status) {
        return Err(format!("OpenCode Go answered {status}."));
    }
    usage_from_body(&body).ok_or_else(|| "OpenCode Go usage could not be read.".into())
}

fn usage_url() -> String {
    override_url("PINNED_OPENCODE_GO_USAGE_URL").unwrap_or_else(|| USAGE_URL.to_owned())
}

/// A gateway override must not send the key to a stranger. Accept HTTPS anywhere, and HTTP only
/// on this machine, where the web sweep and a local proxy run.
fn override_url(name: &str) -> Option<String> {
    allowed_override(std::env::var(name).ok()?.trim())
}

fn allowed_override(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value.trim()).ok()?;
    let host = url.host_str()?.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    (url.scheme() == "https" || (url.scheme() == "http" && loopback)).then(|| url.as_str().trim_end_matches('/').to_owned())
}

/// `rolling` is the 5-hour window, `weekly` the 7-day window, and `monthly` the monthly one.
/// A window that is missing, or that has no percent and reset, is left out.
pub fn usage_from_body(body: &str) -> Option<Usage> {
    let value: Value = serde_json::from_str(body).ok()?;
    let usage = value.get("usage")?.as_object()?;
    Some(Usage {
        five_hour: usage.get("rolling").and_then(window),
        seven_day: usage.get("weekly").and_then(window),
        monthly: usage.get("monthly").and_then(window),
    })
}

fn window(value: &Value) -> Option<LimitWindow> {
    let usage = value.as_object()?;
    let used_percentage = usage.get("percent").and_then(Value::as_f64).filter(|n| n.is_finite())?;
    let resets_at = usage.get("resetsAt").and_then(reset_at)?;
    Some(LimitWindow { used_percentage, resets_at })
}

/// An RFC3339 string, or a number of milliseconds (also seconds, when it is too small to be
/// milliseconds).
fn reset_at(value: &Value) -> Option<u64> {
    match value {
        Value::String(text) => chrono::DateTime::parse_from_rfc3339(text).ok().and_then(|t| u64::try_from(t.timestamp()).ok()),
        Value::Number(number) => unix_number(number.as_f64()?),
        _ => None,
    }
}

fn unix_number(n: f64) -> Option<u64> {
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    let seconds = if n >= 10_000_000_000.0 { n / 1000.0 } else { n };
    Some(seconds.floor() as u64)
}

fn get(url: &str, key: &str, timeout: Duration) -> Result<(u16, String), String> {
    let response = client(timeout)?
        .get(url)
        .bearer_auth(key.trim())
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .map_err(|e| format!("could not reach OpenCode Go: {e}"))?;
    let status = response.status().as_u16();
    let body = response.text().map_err(|e| format!("could not reach OpenCode Go: {e}"))?;
    Ok((status, body))
}

fn get_public(url: &str, timeout: Duration) -> Result<(u16, String), String> {
    let response = client(timeout)?
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .map_err(|e| format!("could not reach OpenCode Go: {e}"))?;
    let status = response.status().as_u16();
    let body = response.text().map_err(|e| format!("could not reach OpenCode Go: {e}"))?;
    Ok((status, body))
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    // The statusline is a fresh process and can reach here before the binary's own install.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::blocking::Client::builder().timeout(timeout).build().map_err(|e| format!("could not reach OpenCode Go: {e}"))
}

/// Skips Claude Code's theme wizard on the first launch in a new home.
pub fn prepare_home(home: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| format!("could not make {}: {e}", home.display()))?;
    crate::first_run::seed(&home.join(".claude.json"), None)
}

/// Drops a Claude login left in a home that is being reused for a Go account. The login lives in
/// the config dir's credentials file and, on macOS, in a keychain entry keyed by that path.
pub fn clear_login(home: &std::path::Path, claude: &std::path::Path, path_env: Option<&str>) {
    let _ = std::fs::remove_file(home.join(".credentials.json"));
    let mut cmd = crate::process::command(claude);
    crate::machine::ChildEnv::new(&[], vec![("CLAUDE_CONFIG_DIR".into(), home.display().to_string())]).apply(&mut cmd);
    cmd.args(["auth", "logout"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(path) = path_env {
        cmd.env("PATH", path);
    }
    let _ = crate::accounts::run(cmd);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    #[test]
    fn usage_from_body_reads_each_window_and_skips_one_that_is_missing() {
        let body = r#"{"usage":{
            "rolling":{"status":"ok","percent":42,"resetsAt":"2026-11-02T23:00:00.000Z"},
            "weekly":{"status":"ok","percent":18.5,"resetsAt":1793923200000},
            "monthly":{"status":"rate-limited","percent":100,"resetsAt":1795996800}
        }}"#;
        assert_eq!(
            usage_from_body(body),
            Some(Usage {
                five_hour: Some(LimitWindow { used_percentage: 42.0, resets_at: 1793660400 }),
                seven_day: Some(LimitWindow { used_percentage: 18.5, resets_at: 1793923200 }),
                monthly: Some(LimitWindow { used_percentage: 100.0, resets_at: 1795996800 }),
            })
        );
        let rolling_only = usage_from_body(r#"{"usage":{"rolling":{"percent":1,"resetsAt":"2026-11-02T23:00:00Z"}}}"#).unwrap();
        assert_eq!(rolling_only.five_hour, Some(LimitWindow { used_percentage: 1.0, resets_at: 1793660400 }));
        assert_eq!((rolling_only.seven_day, rolling_only.monthly), (None, None));
        assert!(usage_from_body(r#"{"usage":{"weekly":{"status":"ok"}}}"#).unwrap().seven_day.is_none());
        assert_eq!(usage_from_body("{}"), None);
        assert_eq!(usage_from_body("nope"), None);
        assert_eq!(usage_from_body(""), None);
    }

    #[test]
    fn models_have_unique_ids_and_the_default_models() {
        let models = models();
        assert_eq!(models.len(), 31);
        let mut ids: Vec<_> = models.iter().map(|model| model.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 31);
        for id in [DEFAULT_OPUS_MODEL, DEFAULT_SONNET_MODEL, DEFAULT_HAIKU_MODEL, SMALL_FAST_MODEL] {
            assert!(models.iter().any(|model| model.id == id), "{id}");
        }
        let kimi = models.iter().find(|model| model.id == "kimi-k3").unwrap();
        assert_eq!((kimi.label.as_str(), kimi.efforts.as_slice()), ("Kimi K3", &["max".to_owned()][..]));
        assert!(models.iter().find(|model| model.id == "kimi-k2.7-code").unwrap().efforts.is_empty());
        assert!(models.iter().flat_map(|model| &model.efforts).all(|effort| CLAUDE_EFFORT.contains(&effort.as_str())), "an effort the CLI rejects");
        assert!(models.iter().find(|model| model.id == "beam-free").unwrap().free, "beam-free costs nothing");
        assert!(models.iter().find(|model| model.id == "iceman-contributor-free").unwrap().free, "iceman costs nothing");
        assert!(models.iter().find(|model| model.id == "longcat-2.5-free").unwrap().free, "longcat-2.5-free costs nothing");
        assert!(models.iter().find(|model| model.id == "hy4-flash-free").unwrap().free, "hy4-flash-free costs nothing");
        assert!(models.iter().any(|model| model.id == "qwen4-max"), "qwen4-max is listed");
        assert!(models.iter().any(|model| model.id == "hy4"), "hy4 is listed");
        assert!(!models.iter().find(|model| model.id == "kimi-k3").unwrap().free, "kimi-k3 is paid");
        let fallback = fallback_models();
        assert_eq!(fallback.len(), models.len());
        assert_eq!(fallback.iter().find(|m| m.slug == "beam-free").unwrap().context, 1_048_576);
        assert!(fallback.iter().find(|m| m.slug == "beam-free").unwrap().free);
        assert_eq!(fallback.iter().find(|m| m.slug == "hy4-flash-free").unwrap().efforts, ["high"], "the shipped table drops none too");
        assert!(fallback.iter().flat_map(|m| &m.efforts).all(|e| CLAUDE_EFFORT.contains(&e.as_str())), "the shipped table drops minimal too");
        let env = env("  oc_sk_test  ");
        assert_eq!(
            env,
            vec![
                (KEY_ENV.to_owned(), "oc_sk_test".into()),
                ("ANTHROPIC_BASE_URL".into(), BASE_URL.into()),
                ("ANTHROPIC_AUTH_TOKEN".into(), "oc_sk_test".into()),
                ("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), DEFAULT_OPUS_MODEL.into()),
                ("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), DEFAULT_SONNET_MODEL.into()),
                ("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), DEFAULT_HAIKU_MODEL.into()),
                ("ANTHROPIC_SMALL_FAST_MODEL".into(), SMALL_FAST_MODEL.into()),
            ]
        );
    }

    fn serve(status: u16, seen: Arc<Mutex<String>>) -> String {
        serve_body(status, "{}", seen)
    }

    fn serve_body(status: u16, body: &'static str, seen: Arc<Mutex<String>>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            *seen.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).into_owned();
            let raw = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(raw.as_bytes());
        });
        format!("http://{address}")
    }

    #[test]
    fn validate_at_accepts_a_key_the_server_accepts_and_names_a_rejection() {
        let seen = Arc::new(Mutex::new(String::new()));
        let base = serve(200, seen.clone());
        assert_eq!(validate_at(&base, "oc_sk_ok"), Ok(()));
        let request = seen.lock().unwrap().clone();
        assert!(request.contains("GET /v1/models"), "{request}");
        assert!(request.to_ascii_lowercase().contains("authorization: bearer oc_sk_ok"), "{request}");
        assert!(request.to_ascii_lowercase().contains(&format!("user-agent: {USER_AGENT}")), "{request}");
        assert_eq!(validate_at(&serve(401, Arc::new(Mutex::new(String::new()))), "nope").unwrap_err(), "OpenCode Go rejected this key.");
        assert_eq!(
            validate_at(&serve(403, Arc::new(Mutex::new(String::new()))), "nope").unwrap_err(),
            "This key's workspace has no OpenCode Go subscription."
        );
        assert_eq!(validate_at(&serve(500, Arc::new(Mutex::new(String::new()))), "nope").unwrap_err(), "OpenCode Go answered 500.");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed = listener.local_addr().unwrap();
        drop(listener);
        let err = validate_at(&format!("http://{closed}"), "oc_sk_ok").unwrap_err();
        assert!(err.starts_with("could not reach OpenCode Go:"), "{err}");
    }

    #[test]
    fn usage_at_reads_the_gateway_and_names_a_refusal() {
        let body = r#"{"usage":{"rolling":{"status":"ok","percent":42,"resetsAt":"2026-11-02T23:00:00.000Z"},"weekly":{"status":"ok","percent":18,"resetsAt":"2026-11-06T00:00:00.000Z"},"monthly":{"status":"ok","percent":80,"resetsAt":"2026-11-30T00:00:00.000Z"}}}"#;
        let seen = Arc::new(Mutex::new(String::new()));
        let url = format!("{}/zen/go/v1/usage", serve_body(200, body, seen.clone()));
        let usage = usage_at(&url, "oc_sk_ok").unwrap();
        assert_eq!(usage.five_hour.map(|w| w.used_percentage), Some(42.0));
        assert_eq!(usage.seven_day.map(|w| w.used_percentage), Some(18.0));
        assert_eq!(usage.monthly.map(|w| w.used_percentage), Some(80.0));
        let request = seen.lock().unwrap().clone();
        assert!(request.contains("GET /zen/go/v1/usage"), "{request}");
        assert!(request.to_ascii_lowercase().contains("authorization: bearer oc_sk_ok"), "{request}");
        assert_eq!(usage_at(&serve_body(401, "{}", Arc::new(Mutex::new(String::new()))), "bad").unwrap_err(), "OpenCode Go answered 401.");
        assert_eq!(usage_at(&serve_body(200, "{}", Arc::new(Mutex::new(String::new()))), "bad").unwrap_err(), "OpenCode Go usage could not be read.");
    }

    #[test]
    fn parse_models_reads_the_gateway_order_and_marks_free_by_price() {
        let body = r#"{"object":"list","data":[
            {"id":"qwen4-max","name":"Qwen4 Max","capabilities":{"reasoning":{"effort":["low","high","xhigh"]}},"limit":{"context":1000000},"cost":{"text":{"input":{"price":1.5},"output":{"price":4.5}}}},
            {"id":"hy4-flash-free","name":"Hy4 flash Free","capabilities":{"reasoning":{"effort":["none","high"]}},"limit":{"context":1048576},"cost":{"text":{"input":{"price":0},"output":{"price":0}}}},
            {"id":"kimi-k2.7-code","name":"Kimi K2.7 Code","limit":{"context":262144},"cost":{"text":{"input":{"price":0.95},"output":{"price":4}}}},
            {"id":"every-level","name":"Every Level","capabilities":{"reasoning":{"effort":["none","low","medium","high","xhigh","max"]}}},
            {"id":"none-only","name":"None Only","capabilities":{"reasoning":{"effort":["none"]}}},
            {"id":"","name":"Nope"},
            {"name":"Missing"}
        ]}"#;
        let models = parse_models(body);
        assert_eq!(models.iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(), ["qwen4-max", "hy4-flash-free", "kimi-k2.7-code", "every-level", "none-only"]);
        assert_eq!(models[0].efforts, ["low", "high", "xhigh"]);
        assert_eq!(models[1].efforts, ["high"], "the gateway's none is not offered");
        assert_eq!((models[0].context, models[0].free), (1_000_000, false));
        assert_eq!((models[1].context, models[1].free, models[1].cost), (1_048_576, true, crate::delegate::Cost::Low));
        assert_eq!(models[3].efforts, ["low", "medium", "high", "xhigh", "max"]);
        assert!(models[4].efforts.is_empty(), "a model whose only level is none offers none");
        assert!(models[2].efforts.is_empty() && !models[2].free);
        assert_eq!(parse_models("{}"), Vec::new());
        assert_eq!(parse_models("nope"), Vec::new());
        let cached = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(cached.path(), serde_json::to_vec(&models).unwrap()).unwrap();
        assert_eq!(load_cached(cached.path()).unwrap().len(), 5);
        std::fs::write(cached.path(), r#"[{"slug":"old-cache","name":"Old Cache","efforts":["none","minimal","high"],"context":1000000,"cost":"high"}]"#).unwrap();
        assert_eq!(load_cached(cached.path()).unwrap()[0].efforts, ["high"], "a cache written before the filter drops none and minimal too");
        assert_eq!(load_cached(std::path::Path::new("/nonexistent-pinned-go-models.json")), None);
    }

    #[test]
    fn a_refresh_is_due_hourly_after_a_success_and_a_minute_after_a_failure() {
        let now = Instant::now();
        let ago = |secs: u64| Some(now - Duration::from_secs(secs));
        assert!(refresh_due(None, false, now), "no attempt yet is due");
        assert!(refresh_due(None, true, now), "no attempt after a failure is due");
        assert!(!refresh_due(ago(300), false, now), "a success 5 minutes ago is not due");
        assert!(refresh_due(ago(3660), false, now), "a success 61 minutes ago is due");
        assert!(!refresh_due(ago(10), true, now), "a failure 10 seconds ago is not due");
        assert!(refresh_due(ago(61), true, now), "a failure 61 seconds ago is due");
    }

    #[test]
    fn gateway_overrides_are_https_or_loopback() {
        assert_eq!(allowed_override("https://opencode.ai/inference/go/"), Some("https://opencode.ai/inference/go".into()));
        assert_eq!(allowed_override(" http://127.0.0.1:8798/ "), Some("http://127.0.0.1:8798".into()));
        assert_eq!(allowed_override("http://localhost:8798"), Some("http://localhost:8798".into()));
        assert_eq!(allowed_override("http://[::1]:8798/"), Some("http://[::1]:8798".into()));
        assert_eq!(allowed_override("http://192.168.1.2:8798"), None);
        assert_eq!(allowed_override("http://localhost.evil.com"), None);
        assert_eq!(allowed_override("ftp://opencode.ai"), None);
        assert_eq!(allowed_override("not a url"), None);
        assert_eq!(allowed_override(""), None);
    }

    #[test]
    fn a_session_gateway_override_passes_the_same_gate() {
        assert_eq!(gateway_from(Some("http://127.0.0.1:55101/")), "http://127.0.0.1:55101");
        assert_eq!(gateway_from(Some("http://192.168.1.2:55101")), BASE_URL);
        assert_eq!(gateway_from(Some("not a url")), BASE_URL);
        assert_eq!(gateway_from(None), BASE_URL);
    }

    #[test]
    fn preparing_a_home_skips_the_theme_wizard_and_keeps_other_settings() {
        let home = std::env::temp_dir().join(format!("pinned-go-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".claude.json"), r#"{"theme":"light","hasCompletedOnboarding":false,"other":1}"#).unwrap();
        prepare_home(&home).unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(home.join(".claude.json")).unwrap()).unwrap();
        assert_eq!(value["hasCompletedOnboarding"], true);
        assert_eq!(value["theme"], "light");
        assert_eq!(value["other"], 1);
        prepare_home(&home).unwrap();
        std::fs::write(home.join(".claude.json"), "not json").unwrap();
        assert!(prepare_home(&home).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    #[cfg(unix)]
    fn clearing_a_login_removes_the_credentials_and_asks_the_cli_to_log_out() {
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("pinned-go-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".credentials.json"), "{}").unwrap();
        let claude = home.join("claude");
        let log = home.join("log");
        std::fs::write(&claude, format!("#!/bin/sh\nprintf '%s|%s' \"$*\" \"$CLAUDE_CONFIG_DIR\" > \"{}\"\n", log.display())).unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        clear_login(&home, &claude, None);
        assert!(!home.join(".credentials.json").exists());
        let said = std::fs::read_to_string(&log).unwrap();
        assert_eq!(said, format!("auth logout|{}", home.display()));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Hits the real usage endpoint. Run with `OPENCODE_GO_KEY` set; `cargo test` skips it.
    #[test]
    #[ignore]
    fn live_usage_reads_opencode_go() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let key = std::env::var("OPENCODE_GO_KEY").expect("OPENCODE_GO_KEY");
        let usage = usage(&key).expect("usage");
        assert!(usage.five_hour.is_some() || usage.seven_day.is_some() || usage.monthly.is_some());
    }
}
