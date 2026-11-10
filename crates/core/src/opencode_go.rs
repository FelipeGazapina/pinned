//! OpenCode Go, as an auth mode of the Claude Code provider. The `claude` CLI talks to Go's
//! Anthropic Messages gateway. The model list is the one `GET /inference/go/v1/models` returned
//! on 2026-11-02. It can change. An id that is not in this list still works when a caller passes
//! it as `target.model`. `ui/gui-composer.js` keeps the same ids, labels, efforts and context
//! windows. Change the two lists together.

use crate::agent::{LimitWindow, Usage};
use crate::delegate::ModelChoice;
use serde_json::Value;
use std::time::Duration;

/// Set on a Go session so the statusline can ask Go for usage. Never a login account.
pub const KEY_ENV: &str = "PINNED_OPENCODE_GO_KEY";
pub const BASE_URL: &str = "https://opencode.ai/inference/go/anthropic";
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

/// `claude --effort` accepts these, so the delegate catalog offers only the gateway's values in
/// this set. `none` and `minimal` stay off the CLI flag.
const SUPPORTED_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

const USER_AGENT: &str = concat!("pinned/", env!("CARGO_PKG_VERSION"));
const NO_EFFORT: &[&str] = &[];

/// `(id, label, efforts)` in the order the gateway listed them.
const MODELS: &[(&str, &str, &[&str])] = &[
    ("deepseek-v4.1-flash", "DeepSeek V4.1 Flash", &["low", "high", "max"]),
    ("deepseek-v4-pro", "DeepSeek V4 Pro (New)", &["high", "max"]),
    ("glm-5.2", "GLM-5.2", &["high", "max"]),
    ("glm-5.3", "GLM-5.3", &["low", "high", "max"]),
    ("grok-4.7", "Grok 4.7", &["low", "medium", "high", "xhigh"]),
    ("muse-spark-1.2-contributor", "Muse Spark 1.2 Contributor", &["minimal", "low", "medium", "high", "xhigh"]),
    ("muse-spark-1.3-contributor", "Muse Spark 1.3 Contributor", &["minimal", "low", "medium", "high", "xhigh"]),
    ("glm-5.3-flash", "GLM-5.3-Flash", &["low", "high", "max"]),
    ("gpt-5.6-luna", "GPT-5.6 Luna", &["none", "low", "medium", "high", "xhigh", "max"]),
    ("gpt-6-luna", "GPT-6 Luna", &["none", "low", "medium", "high", "xhigh", "max"]),
    ("claude-haiku-5-5", "Claude Haiku 5.5", &["low", "medium", "high", "xhigh", "max"]),
    ("hy3", "Hy3", &["none", "low", "high"]),
    ("hy4-preview", "Hy4 preview", &["none", "high"]),
    ("kimi-k2.7-code", "Kimi K2.7 Code", NO_EFFORT),
    ("kimi-k3", "Kimi K3", &["max"]),
    ("mimo-v2.6-flash", "MiMo-V2.6-Flash", NO_EFFORT),
    ("mimo-v2.6-pro", "MiMo-V2.6-Pro", NO_EFFORT),
    ("minimax-m3", "MiniMax-M3", NO_EFFORT),
    ("minimax-m3.1-flash", "MiniMax M3.1 Flash", &["low", "medium", "high", "xhigh", "max"]),
    ("longcat-2.0", "LongCat-2.0", NO_EFFORT),
    ("step-5-preview", "Step 5 Preview", &["low", "medium", "high"]),
    ("beam-free", "Beam Free", &["low", "medium", "high", "xhigh", "max"]),
    ("qwen3.8-max", "Qwen3.8 Max", &["low", "medium", "xhigh"]),
    ("qwen3.8-flash", "Qwen3.8 Flash", &["low", "medium", "xhigh"]),
    ("qwen3.7-plus", "Qwen3.7 Plus", NO_EFFORT),
    ("kimi-k3.1-pro", "Kimi K3.1 Pro", &["low", "high", "max"]),
    ("iceman-contributor-free", "Iceman Contributor Free", NO_EFFORT),
    ("gemini-4-argon", "Gemini 4 Argon", &["low", "medium", "high"]),
];

pub fn models() -> Vec<ModelChoice> {
    MODELS
        .iter()
        .map(|(id, label, efforts)| ModelChoice {
            id: (*id).into(),
            label: (*label).into(),
            efforts: efforts
                .iter()
                .filter(|effort| SUPPORTED_EFFORTS.contains(effort))
                .map(|effort| (*effort).into())
                .collect(),
            cost: crate::delegate::cost_of(id),
        })
        .collect()
}

/// What a Go session adds after pinned has stripped a parent shell's Anthropic variables.
pub fn env(key: &str) -> Vec<(String, String)> {
    let key = key.trim().to_owned();
    vec![
        (KEY_ENV.to_owned(), key.clone()),
        ("ANTHROPIC_BASE_URL".into(), gateway_from(std::env::var("PINNED_OPENCODE_GO_BASE").ok().as_deref())),
        ("ANTHROPIC_AUTH_TOKEN".into(), key),
        ("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), DEFAULT_OPUS_MODEL.into()),
        ("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), DEFAULT_SONNET_MODEL.into()),
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), DEFAULT_HAIKU_MODEL.into()),
        ("ANTHROPIC_SMALL_FAST_MODEL".into(), SMALL_FAST_MODEL.into()),
    ]
}

fn gateway_from(value: Option<&str>) -> String {
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
    cmd.args(["auth", "logout"])
        .env("CLAUDE_CONFIG_DIR", home)
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
        assert_eq!(models.len(), 28);
        let mut ids: Vec<_> = models.iter().map(|model| model.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 28);
        for id in [DEFAULT_OPUS_MODEL, DEFAULT_SONNET_MODEL, DEFAULT_HAIKU_MODEL, SMALL_FAST_MODEL] {
            assert!(models.iter().any(|model| model.id == id), "{id}");
        }
        let kimi = models.iter().find(|model| model.id == "kimi-k3").unwrap();
        assert_eq!((kimi.label.as_str(), kimi.efforts.as_slice()), ("Kimi K3", &["max".to_owned()][..]));
        assert!(models.iter().find(|model| model.id == "kimi-k2.7-code").unwrap().efforts.is_empty());
        assert!(models.iter().flat_map(|model| &model.efforts).all(|effort| SUPPORTED_EFFORTS.contains(&effort.as_str())), "an effort the CLI rejects");
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
