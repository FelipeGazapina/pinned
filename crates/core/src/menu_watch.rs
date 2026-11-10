//! Tells when the user answered the permission or question menu a session waits on, from the
//! session's own screen. Claude Code's hooks report the ask but never the answer, so without this
//! a session reads as waiting on the user until its next hook event, often the turn's end.
//!
//! A watch exists only while a session waits on a menu: it replays the bytes the tab keeps into a
//! terminal of the session's size, then reads each new byte. The menu is answered once it was on
//! the screen and has been gone for a moment, unless Claude Code says the turn was interrupted,
//! which is what a "No" leaves: the session then waits on the user for a prompt.

use std::time::{Duration, Instant};

/// How long the menu must stay gone: a redraw can arrive in two writes, and a question with
/// several parts draws its next menu right after the first is answered.
pub const SETTLE: Duration = Duration::from_millis(700);

/// Rows above the menu's top that a refusal's "Interrupted" line may land on.
const ABOVE: usize = 4;

pub struct MenuWatch {
    parser: vt100::Parser,
    /// The top row of the menu last seen on the screen.
    top: Option<usize>,
    gone_since: Option<Instant>,
}

impl MenuWatch {
    pub fn new(rows: u16, cols: u16, kept: &[u8], now: Instant) -> Self {
        let mut parser = vt100::Parser::new(rows.max(1), cols.max(1), 0);
        parser.process(kept);
        let mut watch = Self { parser, top: None, gone_since: None };
        watch.look(now);
        watch
    }

    pub fn feed(&mut self, bytes: &[u8], now: Instant) {
        self.parser.process(bytes);
        self.look(now);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(rows.max(1), cols.max(1));
    }

    /// The menu was on the screen and has been gone for `SETTLE`, and the turn goes on.
    pub fn answered(&self, now: Instant) -> bool {
        let Some(since) = self.gone_since else { return false };
        if now.saturating_duration_since(since) < SETTLE {
            return false;
        }
        let from = self.top.unwrap_or(0).saturating_sub(ABOVE);
        !self.lines().iter().skip(from).any(|line| line.contains("Interrupted"))
    }

    fn lines(&self) -> Vec<String> {
        let (_, cols) = self.parser.screen().size();
        self.parser.screen().rows(0, cols).collect()
    }

    fn look(&mut self, now: Instant) {
        match menu_top(&self.lines()) {
            Some(top) => {
                self.top = Some(top);
                self.gone_since = None;
            }
            None if self.top.is_some() => {
                self.gone_since.get_or_insert(now);
            }
            None => {}
        }
    }
}

/// The top row of the menu on a screen of `lines`, if one is up: a column of at least two options
/// at the bottom, one marked ❯ or ›, as `menuOn` in ui/terminal-menu.js reads it.
pub fn menu_top(lines: &[String]) -> Option<usize> {
    let lines: Vec<Vec<char>> = lines.iter().map(|line| line.trim_end().chars().collect()).collect();
    let text = |k: usize| lines[k].iter().collect::<String>();
    let hint = |k: usize| {
        let line = text(k);
        ["Esc", "Enter", "Tab", "ctrl+"].iter().any(|word| line.contains(word))
    };
    let chosen = |k: usize| {
        let line = &lines[k];
        let start = line.iter().position(|c| !c.is_whitespace());
        start.is_some_and(|at| matches!(line[at], '❯' | '›') && line.get(at + 1) == Some(&' ') && line.get(at + 2).is_some_and(|c| !c.is_whitespace()))
    };
    let mut end = lines.len().checked_sub(1)?;
    while (lines[end].is_empty() || hint(end)) && !chosen(end) {
        end = end.checked_sub(1)?;
    }
    let floor = end.saturating_sub(16);
    let cursor = (floor..=end).rev().find(|&k| chosen(k))?;
    let at = lines[cursor].iter().position(|c| matches!(c, '❯' | '›'))? + 2;
    let option = |k: usize| {
        let line = &lines[k];
        line.len() > at && line[..at].iter().all(|c| c.is_whitespace() || matches!(c, '❯' | '›')) && !line[at].is_whitespace()
    };
    let options: Vec<usize> = (floor..=end).filter(|&k| option(k)).collect();
    (options.len() >= 2).then(|| options[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    const MENU: &str = "\x1b[2J\x1b[H Bash command\r\n\r\n   cargo build\r\n\r\n Do you want to proceed?\r\n ❯ 1. Yes\r\n   2. Yes, and don't ask again for cargo build commands\r\n   3. No, and tell Claude what to do differently (esc)\r\n";

    fn at(watch: &MenuWatch, after: Duration, from: Instant) -> bool {
        watch.answered(from + after)
    }

    #[test]
    fn a_permission_menu_is_read_from_the_screen() {
        let lines: Vec<String> = ["", " Do you want to proceed?", " ❯ 1. Yes", "   2. No", "", " Esc to cancel"].iter().map(|l| l.to_string()).collect();
        assert_eq!(menu_top(&lines), Some(2));
        let plain: Vec<String> = ["", "› Settings › Archive", "done"].iter().map(|l| l.to_string()).collect();
        assert_eq!(menu_top(&plain), None);
    }

    #[test]
    fn the_menu_is_answered_once_it_stays_gone_and_the_turn_goes_on() {
        let now = Instant::now();
        let mut watch = MenuWatch::new(24, 80, MENU.as_bytes(), now);
        assert!(!at(&watch, Duration::from_secs(5), now), "the menu is still up");
        // Claude Code erases the menu and prints the tool running.
        watch.feed(b"\x1b[5;1H\x1b[J \xe2\x8f\xba Bash(cargo build)\r\n   Running\xe2\x80\xa6\r\n", now);
        assert!(!at(&watch, Duration::from_millis(100), now), "a redraw in two writes is not an answer yet");
        assert!(at(&watch, SETTLE, now));
    }

    #[test]
    fn a_menu_drawn_again_in_time_is_not_answered() {
        let now = Instant::now();
        let mut watch = MenuWatch::new(24, 80, MENU.as_bytes(), now);
        watch.feed(b"\x1b[5;1H\x1b[J", now);
        watch.feed(MENU.as_bytes(), now + Duration::from_millis(200));
        assert!(!at(&watch, Duration::from_secs(5), now));
    }

    #[test]
    fn a_refusal_leaves_the_session_waiting_on_the_user() {
        let now = Instant::now();
        let mut watch = MenuWatch::new(24, 80, MENU.as_bytes(), now);
        watch.feed("\x1b[5;1H\x1b[J   \u{23bf}  Interrupted \u{b7} What should Claude do instead?\r\n\r\n> \r\n".as_bytes(), now);
        assert!(!at(&watch, Duration::from_secs(5), now));
    }

    #[test]
    fn a_screen_where_no_menu_was_seen_is_never_answered() {
        let now = Instant::now();
        let watch = MenuWatch::new(24, 80, b"\x1b[2J\x1b[H garbled", now);
        assert!(!at(&watch, Duration::from_secs(60), now));
    }
}
