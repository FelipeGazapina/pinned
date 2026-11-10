//! Provider accounts that a usage limit stopped, and when to try them again.
//!
//! Two sources say an account is limited. The usage windows an account reports (five hour, seven
//! day, monthly) are the main one: at 99% the account is limited until the window resets. A child
//! that dies on a 429 or a "usage limit exceeded" error is the other: it marks the account here for
//! the providers that report no window, or one that is stale. The state belongs to the machine's
//! account, in a file beside `tasks.json`.
//!
//! A mark with no reset time backs off: 5, 15, 30, then 60 minutes, capped. When the backoff ends
//! the account is not cleared. It goes half open: one delegated task may run on it as the trial,
//! the rest route around. A trial that succeeds clears the mark. One that fails with a limit error
//! marks it again with the next step. The trial is a real task, so the probe spends no quota of its
//! own.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// How long a mark with no reset time lasts, by how many times in a row it was marked.
pub const BACKOFF_MINUTES: [u64; 4] = [5, 15, 30, 60];
/// A trial that never reported back stops holding the account's one trial slot after this long.
const TRIAL_STALE_SECS: u64 = 15 * 60;
const REASON_CHARS: usize = 200;

/// Whether `text`, an error a child ended on, says the account hit a usage limit: a 429, OpenCode
/// Go's "Go usage limit exceeded", or Claude's own usage and spend limit errors.
pub fn is_limit_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["usage limit", "spend limit", "hit your limit", "too many requests", "rate_limit"].iter().any(|mark| lower.contains(mark)) || has_status_429(&lower)
}

fn has_status_429(text: &str) -> bool {
    text.match_indices("429").any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + 3..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// The first line of the error, cut short, with anything that looks like a key left out.
pub fn reason_of(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    let words: Vec<&str> = line.split_whitespace().map(|word| if looks_secret(word) { "…" } else { word }).collect();
    let line = words.join(" ");
    match line.char_indices().nth(REASON_CHARS) {
        Some((cut, _)) => format!("{}…", line[..cut].trim_end()),
        None => line,
    }
}

fn looks_secret(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
    word.starts_with("sk-") || word.starts_with("oc_sk_") || word.starts_with("Bearer")
}

/// The reset time an old Claude usage error carries after a pipe: `... limit reached|1792400000`.
pub fn resume_from(text: &str) -> Option<u64> {
    let (_, tail) = text.split_once('|')?;
    let digits: String = tail.trim().chars().take_while(char::is_ascii_digit).collect();
    if digits.len() == 10 {
        digits.parse().ok()
    } else {
        None
    }
}

fn backoff_secs(strikes: u32) -> u64 {
    let step = usize::try_from(strikes.saturating_sub(1)).unwrap_or(usize::MAX).min(BACKOFF_MINUTES.len() - 1);
    BACKOFF_MINUTES[step] * 60
}

/// One limited account.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Limit {
    /// The first line of the error, or what the usage window said.
    pub reason: String,
    /// Unix milliseconds when it was marked. A run that started before it proves nothing.
    pub since: u64,
    /// Unix seconds when the provider said the limit lifts, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_at: Option<u64>,
    /// How many times in a row it was marked without a success between.
    pub strikes: u32,
    /// Unix seconds until which the account stays blocked. After it, one trial may run.
    pub retry_at: u64,
    /// Unix seconds when the running trial started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial: Option<u64>,
}

impl Limit {
    /// What `orchestrator_capabilities` and a failed delegation tell the agent.
    pub fn note(&self) -> String {
        let when = chrono::DateTime::from_timestamp(i64::try_from(self.retry_at).unwrap_or(i64::MAX), 0).map_or_else(|| self.retry_at.to_string(), |at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        match self.resume_at {
            Some(_) => format!("a usage limit stopped this account: {}. It resumes at {when}.", self.reason),
            None => format!("a usage limit stopped this account: {}. pinned lets one task try it again at {when}.", self.reason),
        }
    }
}

/// What a delegation finds when it asks for an account.
#[derive(Clone, Debug, PartialEq)]
pub enum Admit {
    Open,
    /// The account's backoff ended and this task is its one trial.
    Trial,
    Blocked(Limit),
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub struct ProviderLimits {
    file: Option<PathBuf>,
    state: Mutex<BTreeMap<String, Limit>>,
    /// Unix milliseconds. A test sets it.
    clock: Mutex<Clock>,
}

impl Default for ProviderLimits {
    fn default() -> Self {
        Self { file: None, state: Mutex::default(), clock: Mutex::new(Arc::new(wall_ms)) }
    }
}

fn wall_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl ProviderLimits {
    pub fn load(file: Option<PathBuf>) -> Self {
        let state = file.as_ref().and_then(|file| std::fs::read(file).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Self { file, state: Mutex::new(state), ..Self::default() }
    }

    pub fn set_clock(&self, clock: impl Fn() -> u64 + Send + Sync + 'static) {
        *self.clock.lock().unwrap() = Arc::new(clock);
    }

    fn now_ms(&self) -> u64 {
        (self.clock.lock().unwrap().clone())()
    }

    pub fn now(&self) -> u64 {
        self.now_ms() / 1000
    }

    fn persist(&self, state: &BTreeMap<String, Limit>) {
        let Some(file) = &self.file else { return };
        if let Ok(bytes) = serde_json::to_vec_pretty(state) {
            let _ = crate::fsx::replace(file, &bytes, None, None);
        }
    }

    /// Drops what lifted on its own: a mark whose reset time passed.
    fn lifted(state: &mut BTreeMap<String, Limit>, now: u64) -> bool {
        let before = state.len();
        state.retain(|_, limit| limit.resume_at.is_none_or(|at| at > now));
        state.len() != before
    }

    /// Marks `account` limited when `error` says a limit stopped it. False for any other error.
    pub fn mark(&self, account: &str, error: &str, resume_at: Option<u64>) -> bool {
        if !is_limit_error(error) {
            return false;
        }
        self.mark_reason(account, reason_of(error), resume_at.or_else(|| resume_from(error)));
        true
    }

    pub fn mark_reason(&self, account: &str, reason: String, resume_at: Option<u64>) {
        let (now_ms, now) = (self.now_ms(), self.now());
        let mut state = self.state.lock().unwrap();
        Self::lifted(&mut state, now);
        let resume_at = resume_at.filter(|&at| at > now);
        let strikes = match state.get(account) {
            // A burst of children that started before the first failure counts once.
            Some(old) if old.retry_at > now => old.strikes,
            Some(old) => old.strikes + 1,
            None => 1,
        };
        let retry_at = resume_at.unwrap_or(now + backoff_secs(strikes));
        state.insert(account.to_owned(), Limit { reason, since: now_ms, resume_at, strikes, retry_at, trial: None });
        self.persist(&state);
    }

    /// A run on `account` that started at `started_ms` succeeded. It clears the mark when it
    /// started after it.
    pub fn clear(&self, account: &str, started_ms: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        if !state.get(account).is_some_and(|limit| limit.since <= started_ms) {
            return false;
        }
        state.remove(account);
        self.persist(&state);
        true
    }

    /// The account's mark, with whether it still blocks. None when it is not marked.
    pub fn standing(&self, account: &str) -> Option<(Limit, bool)> {
        let now = self.now();
        let mut state = self.state.lock().unwrap();
        if Self::lifted(&mut state, now) {
            self.persist(&state);
        }
        let limit = state.get(account)?.clone();
        let blocked = limit.retry_at > now || limit.trial.is_some_and(|at| now < at + TRIAL_STALE_SECS);
        Some((limit, blocked))
    }

    /// Asks to run a task on `account`. After a backoff the first asker is the trial.
    pub fn admit(&self, account: &str) -> Admit {
        let now = self.now();
        let mut state = self.state.lock().unwrap();
        if Self::lifted(&mut state, now) {
            self.persist(&state);
        }
        let Some(limit) = state.get_mut(account) else { return Admit::Open };
        if limit.retry_at > now || limit.trial.is_some_and(|at| now < at + TRIAL_STALE_SECS) {
            return Admit::Blocked(limit.clone());
        }
        limit.trial = Some(now);
        self.persist(&state);
        Admit::Trial
    }

    /// Gives a trial slot back when the trial never ran.
    pub fn release_trial(&self, account: &str) {
        let mut state = self.state.lock().unwrap();
        if let Some(limit) = state.get_mut(account) {
            limit.trial = None;
            self.persist(&state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const GO_429: &str = "API Error: Request rejected (429) · Go usage limit exceeded";

    fn at(limits: &ProviderLimits, ms: Arc<AtomicU64>) {
        limits.set_clock(move || ms.load(Ordering::SeqCst));
    }

    #[test]
    fn a_429_and_each_usage_limit_text_is_a_limit_error_and_nothing_else_is() {
        for text in [GO_429, "429 Too Many Requests", "You've hit your monthly spend limit", "Claude AI usage limit reached|1792400000", "{\"error\":\"rate_limit\"}", "You've hit your limit · resets 3pm"] {
            assert!(is_limit_error(text), "{text}");
        }
        for text in ["the agent exited (1)", "API Error: 400 Upstream request failed: Invalid image", "listening on port 4290", "error 1429", "compile error in limit.rs"] {
            assert!(!is_limit_error(text), "{text}");
        }
    }

    #[test]
    fn the_reason_is_the_first_line_cut_short_with_keys_left_out() {
        assert_eq!(reason_of(&format!("\n  {GO_429}\nsecond line")), GO_429);
        assert_eq!(reason_of("429 Bearer abc sk-live-123 and oc_sk_9"), "429 … abc … and …");
        assert_eq!(reason_of(&"x".repeat(500)).chars().count(), REASON_CHARS + 1);
        assert_eq!(resume_from("Claude AI usage limit reached|1792400000"), Some(1792400000));
        assert_eq!(resume_from(GO_429), None);
    }

    #[test]
    fn a_429_with_no_reset_marks_the_account_and_backs_off_5_15_30_60_then_stays_at_60() {
        let limits = ProviderLimits::default();
        let ms = Arc::new(AtomicU64::new(1_000_000_000_000));
        at(&limits, ms.clone());
        assert!(!limits.mark("personal", "the agent exited (1)", None));
        assert_eq!(limits.admit("personal"), Admit::Open);
        assert!(limits.mark("personal", GO_429, None));
        let (limit, blocked) = limits.standing("personal").unwrap();
        assert_eq!((limit.reason.as_str(), limit.resume_at, blocked, limit.retry_at), (GO_429, None, true, 1_000_000_000 + 300));
        for minutes in [5, 15, 30, 60, 60] {
            let (limit, blocked) = limits.standing("personal").unwrap();
            assert!((blocked, limit.retry_at - limit.since / 1000) == (true, minutes * 60), "backoff of {minutes} minutes");
            assert!(matches!(limits.admit("personal"), Admit::Blocked(_)));
            ms.store(limit.retry_at * 1000, Ordering::SeqCst);
            assert_eq!(limits.admit("personal"), Admit::Trial, "after {minutes} minutes one task tries it");
            assert!(matches!(limits.admit("personal"), Admit::Blocked(_)), "only one trial at a time");
            assert!(limits.mark("personal", GO_429, None), "the trial failed with a limit");
        }
    }

    #[test]
    fn a_burst_of_failures_from_children_started_before_the_first_counts_once() {
        let limits = ProviderLimits::default();
        let ms = Arc::new(AtomicU64::new(1_000_000_000_000));
        at(&limits, ms.clone());
        for _ in 0..4 {
            limits.mark("personal", GO_429, None);
        }
        let (limit, _) = limits.standing("personal").unwrap();
        assert_eq!((limit.strikes, limit.retry_at), (1, 1_000_000_000 + 300));
    }

    #[test]
    fn a_reset_time_ends_the_limit_by_itself_on_an_injected_clock() {
        let limits = ProviderLimits::default();
        let ms = Arc::new(AtomicU64::new(1_000_000_000_000));
        at(&limits, ms.clone());
        assert!(limits.mark("claude", "Claude AI usage limit reached|1000003600", None));
        let (limit, blocked) = limits.standing("claude").unwrap();
        assert_eq!((limit.resume_at, blocked, limit.retry_at), (Some(1_000_003_600), true, 1_000_003_600));
        ms.store(1_000_003_599_000, Ordering::SeqCst);
        assert!(limits.standing("claude").is_some());
        ms.store(1_000_003_600_000, Ordering::SeqCst);
        assert_eq!((limits.standing("claude"), limits.admit("claude")), (None, Admit::Open), "the reset cleared it with no trial");
    }

    #[test]
    fn only_a_run_that_started_after_the_mark_clears_it() {
        let limits = ProviderLimits::default();
        let ms = Arc::new(AtomicU64::new(1_000_000_000_000));
        at(&limits, ms.clone());
        limits.mark("personal", GO_429, None);
        assert!(!limits.clear("personal", 999_999_999_000), "a run that began before the mark");
        assert!(limits.standing("personal").is_some());
        assert!(limits.clear("personal", 1_000_000_000_000));
        assert_eq!(limits.admit("personal"), Admit::Open);
        limits.mark("personal", GO_429, None);
        assert_eq!(limits.standing("personal").unwrap().0.strikes, 1, "a success reset the count");
    }

    #[test]
    fn the_state_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("pinned-limits-{}", wall_ms()));
        let file = dir.join("provider-limits.json");
        let limits = ProviderLimits::load(Some(file.clone()));
        limits.mark("personal", GO_429, None);
        let again = ProviderLimits::load(Some(file));
        assert_eq!(again.standing("personal").map(|(limit, _)| limit.reason), Some(GO_429.to_owned()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
