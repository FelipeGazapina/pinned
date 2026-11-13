//! Notices that the machine slept, and which failed turns the sleep cut. No platform signal is read:
//! a wall-clock jump between two ticks is the sleep. Every time comes in as an argument, so a test
//! never sleeps.

use crate::wire::CutBySleep;
use std::path::Path;

/// Seconds between the ticks the window is measured from.
pub const TICK_SECS: u64 = 10;
/// A gap between two ticks longer than this is a sleep.
pub const JUMP_SECS: u64 = 60;
/// A turn's error can land this long before the sleep: the network goes first.
pub const BEFORE_SECS: u64 = 60;
/// Or this long after the wake: the dead socket is only noticed on the first read.
pub const AFTER_SECS: u64 = 120;

/// The last tick and the last sleep seen.
#[derive(Default, Debug, PartialEq)]
pub struct Watch {
    last_tick: Option<u64>,
    window: Option<CutBySleep>,
}

impl Watch {
    pub fn with_window(window: Option<CutBySleep>) -> Self {
        Watch { last_tick: None, window }
    }

    pub fn window(&self) -> Option<CutBySleep> {
        self.window
    }

    /// Called as often as the caller likes; counts one tick per `TICK_SECS`. Returns the window when
    /// this tick found the machine had slept.
    pub fn tick(&mut self, now: u64) -> Option<CutBySleep> {
        let last = *self.last_tick.get_or_insert(now);
        if now < last + TICK_SECS && now >= last {
            return None;
        }
        self.last_tick = Some(now);
        if now > last + JUMP_SECS {
            self.window = Some(CutBySleep { slept_at: last, woke_at: now });
            return self.window;
        }
        None
    }
}

/// Whether a failure text is an API error: a line of it starts the way Claude Code renders one.
pub fn is_api_error(text: &str) -> bool {
    text.lines().any(|line| line.trim_start().starts_with("API Error"))
}

/// The window a failed turn was cut by: an API error with no usage limit, whose time falls inside
/// the last sleep with its margins.
pub fn cut_by(window: Option<CutBySleep>, api_error: bool, resume_at: Option<u64>, updated: u64) -> Option<CutBySleep> {
    let w = window?;
    (api_error && resume_at.is_none() && updated + BEFORE_SECS >= w.slept_at && updated <= w.woke_at + AFTER_SECS).then_some(w)
}

pub fn load(file: &Path) -> Option<CutBySleep> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

pub fn save(file: &Path, window: CutBySleep) {
    if let Ok(bytes) = serde_json::to_vec(&window) {
        let _ = std::fs::write(file, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jump_of_more_than_a_minute_between_ticks_is_a_sleep() {
        let mut w = Watch::default();
        assert_eq!(w.tick(1000), None);
        assert_eq!(w.tick(1010), None);
        assert_eq!(w.tick(1070), None, "60 s is a slow tick, not a sleep");
        assert_eq!(w.tick(1200), Some(CutBySleep { slept_at: 1070, woke_at: 1200 }));
        assert_eq!(w.window(), Some(CutBySleep { slept_at: 1070, woke_at: 1200 }));
        assert_eq!(w.tick(1210), None);
        assert_eq!(w.window().unwrap().woke_at, 1200, "the last window stays");
    }

    #[test]
    fn a_clock_set_back_is_not_a_sleep() {
        let mut w = Watch::default();
        w.tick(5000);
        assert_eq!(w.tick(100), None);
        assert_eq!(w.window(), None);
    }

    #[test]
    fn only_an_api_error_with_no_limit_inside_the_window_is_cut() {
        let w = Some(CutBySleep { slept_at: 1000, woke_at: 5000 });
        assert!(cut_by(w, true, None, 3000).is_some(), "inside");
        assert!(cut_by(w, true, None, 940).is_some(), "60 s before the sleep");
        assert!(cut_by(w, true, None, 5120).is_some(), "120 s after the wake");
        assert!(cut_by(w, true, None, 939).is_none());
        assert!(cut_by(w, true, None, 5121).is_none());
        assert!(cut_by(w, true, Some(9000), 3000).is_none(), "a usage limit has its own path");
        assert!(cut_by(w, false, None, 3000).is_none(), "not an API error");
        assert!(cut_by(None, true, None, 3000).is_none(), "no sleep seen");
    }

    #[test]
    fn the_api_error_test_is_failure_texts() {
        assert!(is_api_error("API Error: 529 Overloaded\nrate_limit"));
        assert!(is_api_error("fetch failed\n  API Error: Connection error"));
        assert!(!is_api_error("rate_limit"));
        assert!(!is_api_error("I got an API Error earlier"));
    }

    #[test]
    fn the_window_survives_a_reload() {
        let file = std::env::temp_dir().join(format!("pinned-sleep-test-{}.json", std::process::id()));
        let window = CutBySleep { slept_at: 1792400000, woke_at: 1792420000 };
        assert_eq!(load(&file), None);
        save(&file, window);
        assert_eq!(load(&file), Some(window));
        let _ = std::fs::remove_file(file);
    }
}
