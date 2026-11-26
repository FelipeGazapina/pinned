//! `pinned-server hub --dev`: a hub on this machine, against local stand-ins, with its timers
//! shortened by `PINNED_DEV_TIME_SCALE`. `check` refuses a start that could reach a real service.

use crate::boxes::Policy;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// Slack's API base for a dev hub. `check` requires it to be a loopback root; `hub` then points Slack at it.
const SLACK_BASE_ENV: &str = "PINNED_SLACK_API_BASE";
const TIME_SCALE_ENV: &str = "PINNED_DEV_TIME_SCALE";
const MAX_TIME_SCALE: u32 = 3600;

/// How the hub's timers are shortened. `time_scale` 1 is real time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dev {
    pub time_scale: u32,
    /// The normalised Slack API root (`slack::dev_root`) a dev hub sends Slack calls to. `None` when off.
    pub slack_root: Option<String>,
}

impl Dev {
    pub const OFF: Dev = Dev { time_scale: 1, slack_root: None };

    /// Real duration divided by time_scale, never below 1 second.
    pub fn scale(&self, d: Duration) -> Duration {
        (d / self.time_scale).max(Duration::from_secs(1))
    }

    /// `policy` with every duration scaled. Each field is named, so a new duration fails to build here.
    pub fn policy(&self, policy: &Policy) -> Policy {
        Policy {
            boot_wait: self.scale(policy.boot_wait),
            idle: self.scale(policy.idle),
            done: self.scale(policy.done),
            exited: self.scale(policy.exited),
            tab_gone: self.scale(policy.tab_gone),
            first_turn: self.scale(policy.first_turn),
            offline: self.scale(policy.offline),
            max_life: self.scale(policy.max_life),
            cooldown: self.scale(policy.cooldown),
            tick: self.scale(policy.tick),
            sweep: self.scale(policy.sweep),
            max_open: policy.max_open,
        }
    }
}

/// Checks a dev start. `env` is a lookup so tests pass a map. A refusal names the kind of value
/// that is wrong, never the value, since the database URL may carry a password.
/// A release build is refused first: the dev stack is only meant for a debug build, and a binary of
/// unknown build must not be started on keys that could reach a real service.
pub fn check(db_url: &str, listen: &str, public_url: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<Dev, String> {
    check_build(cfg!(debug_assertions))?;
    check_db(db_url)?;
    if !loopback_listen(listen) {
        return Err("--listen must bind a loopback address, not a wildcard or remote one".into());
    }
    loopback_url("--public-url", public_url)?;
    let box_name = pinned_core::upstash::BASE_URL_ENV;
    let box_base = env(box_name).filter(|v| !v.is_empty()).ok_or_else(|| format!("{box_name} is not set"))?;
    loopback_url(box_name, &box_base)?;
    let slack_base = env(SLACK_BASE_ENV).filter(|v| !v.is_empty()).ok_or_else(|| format!("{SLACK_BASE_ENV} is not set"))?;
    let slack_root = pinned_core::slack::dev_root(&slack_base).ok_or_else(|| format!("{SLACK_BASE_ENV} must be an http or https URL on this machine"))?;
    let time_scale = match env(TIME_SCALE_ENV) {
        None => 1,
        Some(raw) => match raw.parse::<u32>() {
            Ok(n) if (1..=MAX_TIME_SCALE).contains(&n) => n,
            Ok(_) => return Err(format!("{TIME_SCALE_ENV} is outside 1 to {MAX_TIME_SCALE}")),
            Err(_) => return Err(format!("{TIME_SCALE_ENV} is not a whole number")),
        },
    };
    Ok(Dev { time_scale, slack_root: Some(slack_root) })
}

/// `debug` is whether this binary was built with debug assertions (`cargo build` without `--release`).
fn check_build(debug: bool) -> Result<(), String> {
    if debug {
        Ok(())
    } else {
        Err("this is a release build: --dev needs a debug build (cargo build -p pinned-server, without --release)".into())
    }
}

fn check_db(db_url: &str) -> Result<(), String> {
    let url = url::Url::parse(db_url).map_err(|_| "--db is not a postgres:// URL".to_owned())?;
    if !matches!(url.scheme(), "postgres" | "postgresql") {
        return Err("--db must use the postgres:// scheme".into());
    }
    let host = url.host_str().ok_or("--db names no host")?;
    if !is_loopback_host(host) {
        return Err("--db host is not loopback".into());
    }
    // A host given in the query overrides the one before it, so it is held to the same rule.
    for (key, value) in url.query_pairs() {
        if matches!(key.as_ref(), "host" | "hostaddr") && !value.split(',').all(is_loopback_host) {
            return Err("--db host parameter is not loopback".into());
        }
    }
    Ok(())
}

fn loopback_listen(listen: &str) -> bool {
    if let Ok(addr) = listen.parse::<SocketAddr>() {
        return addr.ip().is_loopback();
    }
    match listen.rsplit_once(':') {
        Some((host, port)) => host.eq_ignore_ascii_case("localhost") && port.parse::<u16>().is_ok(),
        None => false,
    }
}

fn loopback_url(name: &str, value: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(|_| format!("{name} is not an http or https URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{name} is not an http or https URL"));
    }
    if !url.host_str().is_some_and(is_loopback_host) {
        return Err(format!("{name} host is not loopback"));
    }
    Ok(())
}

/// Whether `host`, as `Url::host_str` spells it, is this machine: `localhost` or a loopback IP.
fn is_loopback_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost") || bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::{check, check_build, Dev};
    use crate::boxes::Policy;
    use std::time::Duration;

    const DB: &str = "postgres://x@127.0.0.1/p";
    const LISTEN: &str = "127.0.0.1:0";
    const PUBLIC: &str = "http://127.0.0.1:1";
    const BOX: (&str, &str) = ("UPSTASH_BOX_BASE_URL", "http://127.0.0.1:9");
    const SLACK: (&str, &str) = ("PINNED_SLACK_API_BASE", "http://localhost:9");

    fn run(db: &str, listen: &str, public: &str, vars: &[(&str, &str)]) -> Result<Dev, String> {
        let env = |key: &str| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string());
        check(db, listen, public, &env)
    }

    #[test]
    fn refuses_a_build_without_debug_assertions() {
        assert_eq!(check_build(false), Err("this is a release build: --dev needs a debug build (cargo build -p pinned-server, without --release)".into()));
        assert_eq!(check_build(true), Ok(()));
    }

    #[test]
    fn this_test_build_is_a_debug_build_so_check_runs_on_to_the_rest() {
        assert!(cfg!(debug_assertions));
        assert_eq!(run(DB, LISTEN, PUBLIC, &[SLACK]), Err("UPSTASH_BOX_BASE_URL is not set".into()));
    }

    #[test]
    fn accepts_loopback_and_reads_the_time_scale() {
        assert_eq!(run(DB, LISTEN, PUBLIC, &[BOX, SLACK, ("PINNED_DEV_TIME_SCALE", "60")]), Ok(Dev { time_scale: 60, slack_root: Some("http://localhost:9/".into()) }));
        assert_eq!(run(DB, LISTEN, PUBLIC, &[BOX, SLACK]), Ok(Dev { time_scale: 1, slack_root: Some("http://localhost:9/".into()) }), "real time when unset");
    }

    #[test]
    fn refuses_a_remote_database_host_without_echoing_it() {
        let err = run("postgres://x:secret@db.example.com/p", LISTEN, PUBLIC, &[BOX, SLACK]).unwrap_err();
        assert_eq!(err, "--db host is not loopback");
        assert!(!err.contains("secret") && !err.contains("example"));
    }

    #[test]
    fn refuses_a_database_host_parameter_that_is_remote() {
        assert_eq!(run("postgres://x@127.0.0.1/p?host=db.example.com", LISTEN, PUBLIC, &[BOX, SLACK]), Err("--db host parameter is not loopback".into()));
    }

    #[test]
    fn refuses_a_wildcard_listen_address() {
        assert_eq!(run(DB, "0.0.0.0:0", PUBLIC, &[BOX, SLACK]), Err("--listen must bind a loopback address, not a wildcard or remote one".into()));
    }

    #[test]
    fn refuses_a_public_url_on_a_remote_host() {
        assert_eq!(run(DB, LISTEN, "https://example.com", &[BOX, SLACK]), Err("--public-url host is not loopback".into()));
    }

    #[test]
    fn refuses_a_missing_box_base_url() {
        assert_eq!(run(DB, LISTEN, PUBLIC, &[SLACK]), Err("UPSTASH_BOX_BASE_URL is not set".into()));
    }

    #[test]
    fn refuses_a_slack_base_on_slack_com() {
        assert_eq!(run(DB, LISTEN, PUBLIC, &[BOX, ("PINNED_SLACK_API_BASE", "https://slack.com/api")]), Err("PINNED_SLACK_API_BASE must be an http or https URL on this machine".into()));
    }

    #[test]
    fn refuses_a_database_that_is_not_a_url() {
        assert_eq!(run("not a url", LISTEN, PUBLIC, &[BOX, SLACK]), Err("--db is not a postgres:// URL".into()));
    }

    #[test]
    fn refuses_a_database_with_another_scheme() {
        assert_eq!(run("mysql://x@127.0.0.1/p", LISTEN, PUBLIC, &[BOX, SLACK]), Err("--db must use the postgres:// scheme".into()));
    }

    #[test]
    fn refuses_a_missing_slack_base() {
        assert_eq!(run(DB, LISTEN, PUBLIC, &[BOX]), Err("PINNED_SLACK_API_BASE is not set".into()));
    }

    #[test]
    fn refuses_a_box_base_on_a_remote_host() {
        assert_eq!(run(DB, LISTEN, PUBLIC, &[("UPSTASH_BOX_BASE_URL", "https://box.example.com"), SLACK]), Err("UPSTASH_BOX_BASE_URL host is not loopback".into()));
    }

    #[test]
    fn returns_the_normalised_slack_root() {
        let dev = run(DB, LISTEN, PUBLIC, &[BOX, ("PINNED_SLACK_API_BASE", "http://localhost:9/api")]).unwrap();
        assert_eq!(dev.slack_root.as_deref(), Some("http://localhost:9/api/"));
    }

    #[test]
    fn refuses_a_time_scale_of_zero() {
        assert_eq!(run(DB, LISTEN, PUBLIC, &[BOX, SLACK, ("PINNED_DEV_TIME_SCALE", "0")]), Err("PINNED_DEV_TIME_SCALE is outside 1 to 3600".into()));
    }

    #[test]
    fn refuses_a_time_scale_above_an_hour() {
        assert_eq!(run(DB, LISTEN, PUBLIC, &[BOX, SLACK, ("PINNED_DEV_TIME_SCALE", "99999")]), Err("PINNED_DEV_TIME_SCALE is outside 1 to 3600".into()));
    }

    #[test]
    fn refuses_a_time_scale_that_is_not_a_number() {
        assert_eq!(run(DB, LISTEN, PUBLIC, &[BOX, SLACK, ("PINNED_DEV_TIME_SCALE", "x")]), Err("PINNED_DEV_TIME_SCALE is not a whole number".into()));
    }

    #[test]
    fn scale_divides_real_time_and_never_goes_below_one_second() {
        assert_eq!(Dev { time_scale: 60, slack_root: None }.scale(Duration::from_secs(30 * 60)), Duration::from_secs(30));
        assert_eq!(Dev { time_scale: 60, slack_root: None }.scale(Duration::from_secs(2 * 60 * 60)), Duration::from_secs(120));
        assert_eq!(Dev { time_scale: 3600, slack_root: None }.scale(Duration::from_secs(10)), Duration::from_secs(1));
    }

    #[test]
    fn policy_scales_every_duration_and_keeps_the_cap() {
        let policy = Dev { time_scale: 60, slack_root: None }.policy(&Policy::default());
        assert_eq!(policy.boot_wait, Duration::from_secs(5));
        assert_eq!(policy.idle, Duration::from_secs(10));
        assert_eq!(policy.max_life, Duration::from_secs(240));
        assert_eq!(policy.tick, Duration::from_secs(1));
        assert_eq!(policy.max_open, 8);
    }
}
