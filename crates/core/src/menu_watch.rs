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
/// at the bottom, one marked ❯ or ›, numbered or not, read by the rules of `menuOn` in
/// ui/terminal-menu.js, which the UI's own tests pin against screens of real Claude Code versions.
pub fn menu_top(lines: &[String]) -> Option<usize> {
    let lines: Vec<Vec<char>> = lines.iter().map(|line| line.trim_end().chars().collect()).collect();
    if let Some(row) = lines.iter().rposition(|line| preview_row(line)) {
        if let Some(top) = read_menu(&without_preview(&lines, row), Some(row)) {
            return Some(top);
        }
    }
    read_menu(&lines, None)
}

fn read_menu(lines: &[Vec<char>], preview_row: Option<usize>) -> Option<usize> {
    let preview = preview_row.is_some();
    let blank = |chars: &[char]| chars.iter().all(|c| c.is_whitespace());
    let hint = |k: usize| hint_line(&lines[k]);
    let hint_tail = |k: usize| k > 0 && lines[k].first().is_some_and(|c| !c.is_whitespace()) && hint(k - 1);
    let mut end = lines.len().checked_sub(1)?;
    while (blank(&lines[end]) || hint(end) || hint_tail(end)) && cursor_at(&lines[end]).is_none() {
        end = end.checked_sub(1)?;
    }
    let floor = end.saturating_sub(16);
    let cursor = (floor..=end).rev().find(|&k| chosen(&lines[k]))?;
    let at = cursor_at(&lines[cursor])? + 2;
    let option = |k: usize| {
        let line = &lines[k];
        let mut before: Vec<char> = line.iter().take(at).copied().collect();
        if let Some(mark) = cursor_at(&before) {
            before.remove(mark);
        }
        line.len() > at && blank(&before) && !line[at].is_whitespace()
    };
    let continued = |k: usize| lines[k].len() > at && blank(&lines[k][..=at]);
    let number = |k: usize| -> Option<usize> {
        let text: String = lines[k].iter().skip(at).collect();
        let text = text.trim();
        let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
        let rest = &text[digits.len()..];
        (!digits.is_empty() && rest.starts_with('.') && rest[1..].starts_with(char::is_whitespace)).then(|| digits.parse().ok()).flatten()
    };
    let numbered = preview || number(cursor).is_some();
    let rule = |k: usize| numbered && rule_line(&lines[k]);
    let mut first = cursor;
    while first > 0 && (option(first - 1) || continued(first - 1) || (rule(first - 1) && first - 1 > 0 && option(first - 2))) {
        first -= 1;
    }
    while !option(first) {
        first += 1;
    }
    if preview_row.is_some_and(|row| row != first) {
        return None;
    }
    let mut options = 0;
    for k in first..=end {
        if option(k) {
            let unnumbered_last = preview && number(k).is_none() && k > 0 && rule(k - 1);
            if numbered && !unnumbered_last && number(k) != Some(options + 1) {
                return None;
            }
            options += 1;
        } else if continued(k) && options > 0 {
        } else if !rule(k) {
            return None;
        }
    }
    (options >= 2).then_some(first)
}

/// Where ❯ or › stands with a space after it, as `CURSOR` matches it.
fn cursor_at(line: &[char]) -> Option<usize> {
    line.windows(2).position(|pair| matches!(pair[0], '❯' | '›') && pair[1] == ' ')
}

/// The chosen option: ❯ or › first on the line, then a space and the option.
fn chosen(line: &[char]) -> bool {
    let Some(at) = line.iter().position(|c| !c.is_whitespace()) else { return false };
    matches!(line[at], '❯' | '›') && line.get(at + 1) == Some(&' ') && line.get(at + 2).is_some_and(|c| !c.is_whitespace())
}

/// A line of Claude Code's key hints: Esc, Enter or Tab as a word, or a ctrl+ key.
fn hint_line(line: &[char]) -> bool {
    let text: String = line.iter().collect();
    text.split(|c: char| !c.is_alphanumeric()).any(|word| matches!(word, "Esc" | "Enter" | "Tab")) || text.match_indices("ctrl+").any(|(at, _)| text[at + 5..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// A rule of at least eight ─ or ━.
fn rule_line(line: &[char]) -> bool {
    let text: String = line.iter().collect();
    let text = text.trim();
    text.chars().count() >= 8 && text.chars().all(|c| matches!(c, '─' | '━'))
}

/// The row of a question's first option when a preview box is drawn beside it.
fn preview_row(line: &[char]) -> bool {
    let text: String = line.iter().collect();
    let rest = text.trim_start_matches(|c: char| c.is_whitespace() || c == '❯' || c == '›');
    let Some(rest) = rest.strip_prefix("1.") else { return false };
    if !rest.starts_with(char::is_whitespace) {
        return false;
    }
    let Some(pane) = rest.rfind('┌') else { return false };
    let (head, tail) = rest.split_at(pane);
    let inner = tail.trim_end().trim_start_matches('┌');
    let box_top = inner.strip_suffix('┐').is_some_and(|dashes| !dashes.is_empty() && dashes.chars().all(|c| c == '─'));
    box_top && head.ends_with(char::is_whitespace) && head.trim_end().chars().last().is_some_and(|c| !c.is_whitespace())
}

/// The screen with the preview box beside the options taken out, as `withoutPreview` does.
fn without_preview(lines: &[Vec<char>], top: usize) -> Vec<Vec<char>> {
    let Some(pane) = lines[top].iter().rposition(|c| *c == '┌') else { return lines.to_vec() };
    let mut out = lines[..top].to_vec();
    for line in &lines[top..] {
        let left: Vec<char> = line.iter().take(pane).copied().collect();
        if left.iter().all(|c| c.is_whitespace()) {
            continue;
        }
        if line.get(pane).is_some_and(|c| matches!(c, '┌' | '│' | '└')) {
            let mut kept = left;
            while kept.last().is_some_and(|c| c.is_whitespace()) {
                kept.pop();
            }
            out.push(kept);
        } else {
            out.push(line.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MENU: &str = "\x1b[2J\x1b[H Bash command\r\n\r\n   cargo build\r\n\r\n Do you want to proceed?\r\n ❯ 1. Yes\r\n   2. Yes, and don't ask again for cargo build commands\r\n   3. No, and tell Claude what to do differently (esc)\r\n";

    fn at(watch: &MenuWatch, after: Duration, from: Instant) -> bool {
        watch.answered(from + after)
    }

    fn screen(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    /// The screens ui/terminal-menu.test.js reads, from real Claude Code versions: the watch must
    /// see a menu exactly where the UI does.
    #[test]
    fn menus_are_read_as_the_ui_reads_them() {
        let rule = "─".repeat(100);
        let pane = "─".repeat(42);
        let plain = [rule.as_str(), " ☐ Direction ", "Which way should the sweep go?", "❯ 1. North route", "     Go up the hill.", "  2. South route", "     Go down to the river.", "  3. Type something.", rule.as_str(), "  4. Chat about this", "Enter to select · ↑/↓ to navigate · Esc to cancel"];
        assert_eq!(menu_top(&screen(&plain)), Some(3));
        let permission = ["⏺ ┌──────────┐", "  │ client   │", "  └──────────┘", rule.as_str(), " Bash command", "   echo sweep", " Do you want to proceed?", " ❯ 1. Yes", "   2. Yes, and don't ask again for echo commands in this folder", "   3. No, and tell Claude what to do differently (esc)", "", " Esc to cancel · Tab to amend"];
        assert_eq!(menu_top(&screen(&permission)), Some(7));
        let row1 = format!("❯ 1. North route                  ┌{pane}┐");
        let bottom = format!("                                  └{pane}┘");
        let preview = ["  ⎿  sweep", "", rule.as_str(), " ☐ Direction", "", "Which way should the sweep go?", "", row1.as_str(), "  2. South route                  │ North                                    │", "                                  │   step one                               │", bottom.as_str(), "", "                                  Notes: press n to add notes", "", rule.as_str(), "  Chat about this", "", "Enter to select · ↑/↓ to navigate · n to add notes · Esc to cancel"];
        assert_eq!(menu_top(&screen(&preview)), Some(7));
        let narrow_rule = "─".repeat(43);
        let narrow = ["❯ check the sweep", "", narrow_rule.as_str(), " ☐ Direction", "", "Which way should the sweep go?", "", "❯ 1. North route", "     Walk the north side first.", "  2. South route", "     Walk the south side first.", "  3. Type something.", narrow_rule.as_str(), "  4. Chat about this", "", "Enter to select · ↑/↓ to navigate · Esc to", "cancel"];
        assert_eq!(menu_top(&screen(&narrow)), Some(7));

        let draft = ["⏺ Done.", "", rule.as_str(), "❯ 1. fix the bug", rule.as_str(), "  ⏵⏵ accept edits on (shift+tab to cycle)"];
        assert_eq!(menu_top(&screen(&draft)), None, "a draft that starts with a number is no menu");
        assert_eq!(menu_top(&screen(&[rule.as_str(), "Pick one", "❯ 1. One", "  3. Three", "Enter to select"])), None, "numbers that skip are no menu");
        let idle = ["  Both are in Settings › Archive and can be brought back.", "", rule.as_str(), "❯", rule.as_str(), "  ⏵⏵ bypass permissions on (shift+tab to cycle)"];
        assert_eq!(menu_top(&screen(&idle)), None, "an idle prompt box is no menu");
        let running = ["⏺ Bash(cargo build)", "  ⎿  Running…", "", rule.as_str(), "❯ ", rule.as_str(), "  ⏵⏵ accept edits on (shift+tab to cycle)"];
        assert_eq!(menu_top(&screen(&running)), None, "a running turn is no menu");
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
