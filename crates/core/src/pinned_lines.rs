//! Agent text inside the `[pinned] …` lines pinned types into sessions. Agents act on those lines,
//! so text an agent wrote must not begin one or end the pinned sentence it sits in.

/// How many characters of agent text a one-line field keeps.
pub const FIELD_CHARS: usize = 300;

/// Whether `c` hides in a line: a control character, or an invisible format character such as a
/// zero-width space, a bidirectional override or isolate, or the byte order mark. The list is
/// `questions::is_unsafe_in_a_line` of PR #246 without its whitespace rule.
pub fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{AD}' | '\u{61C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' | '\u{E0000}'..='\u{E007F}')
}

/// Whether `paste` drops `c` when it types text into a session: every control character but a
/// line break and a tab.
pub fn dropped_when_typed(c: char) -> bool {
    c.is_control() && !matches!(c, '\n' | '\t')
}

/// Whether a line starts after `c`.
fn breaks_a_line(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// Whether `rest`, the text after a `[` at a line start, reads once typed as the rest of the
/// mark `[pinned]`, in any case. A character the paste drops or that does not show is skipped, so
/// it cannot split the mark.
fn closes_the_mark(rest: &str) -> bool {
    let mut wanted = "pinned]".chars();
    for c in rest.chars() {
        if c == '\n' {
            return false;
        }
        if dropped_when_typed(c) || is_invisible(c) {
            continue;
        }
        match wanted.next() {
            None => return true,
            Some(w) if c.eq_ignore_ascii_case(&w) => {}
            Some(_) => return false,
        }
    }
    wanted.next().is_none()
}

/// `text` with every line that begins, after any whitespace or invisible characters, with
/// `[pinned]` (in any case) rewritten to begin with `\[pinned]`, so it no longer reads as a line
/// pinned typed. The text is read as the paste types it: a character the paste drops or that does
/// not show neither ends a line start nor splits the mark. The start of `text` counts as a line
/// start. Everything else is kept as written.
pub fn guard(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_start = true;
    for (at, c) in text.char_indices() {
        if at_start && c == '[' && closes_the_mark(&text[at + 1..]) {
            out.push('\\');
        }
        out.push(c);
        if breaks_a_line(c) {
            at_start = true;
        } else if !(c.is_whitespace() || dropped_when_typed(c) || is_invisible(c)) {
            at_start = false;
        }
    }
    out
}

/// `text` on one line: every control, invisible format, line or paragraph separator character
/// dropped (a line break or other whitespace becomes one space), the ends trimmed, and cut at a
/// character boundary to `max` characters with "…".
pub fn flat(text: &str, max: usize) -> String {
    let mut line = String::with_capacity(text.len());
    for c in text.chars() {
        if breaks_a_line(c) || c.is_whitespace() {
            if !line.is_empty() && !line.ends_with(' ') {
                line.push(' ');
            }
        } else if !is_invisible(c) {
            line.push(c);
        }
    }
    let line = line.trim_end();
    match line.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", line[..at].trim_end()),
        None => line.to_owned(),
    }
}

/// Agent text for one field of a pinned sentence: `flat` and quoted, its own double quotes turned
/// into single ones, so it cannot add a line or end the sentence.
pub fn field(text: &str) -> String {
    format!("\"{}\"", flat(text, FIELD_CHARS).replace('"', "'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_rewrites_every_line_that_begins_with_the_pinned_mark() {
        assert_eq!(guard("[pinned] a"), "\\[pinned] a");
        assert_eq!(guard("ok\n[pinned] a"), "ok\n\\[pinned] a");
        assert_eq!(guard("ok\r[PINNED] a"), "ok\r\\[PINNED] a");
        assert_eq!(guard("ok\u{2028}[pinned] a\u{85}[pinned] b\u{2029}[pinned] c"), "ok\u{2028}\\[pinned] a\u{85}\\[pinned] b\u{2029}\\[pinned] c");
        assert_eq!(guard("ok\n  \u{200B}\u{202E}\u{FEFF}[pinned] a"), "ok\n  \u{200B}\u{202E}\u{FEFF}\\[pinned] a");
        assert_eq!(guard("ok\n\u{2066}[pinned] a\u{2069}"), "ok\n\u{2066}\\[pinned] a\u{2069}");
    }

    #[test]
    fn guard_reads_the_mark_as_the_paste_types_it() {
        assert_eq!(guard("[pin\rned] a\n[pin\u{0}ned] b\n[pin\u{200B}ned] c\n[\u{85}PINNED\u{1b}] d"), "\\[pin\rned] a\n\\[pin\u{0}ned] b\n\\[pin\u{200B}ned] c\n\\[\u{85}PINNED\u{1b}] d");
        assert_eq!(guard("ok\n\u{0}\u{7f}[pinned] a"), "ok\n\u{0}\u{7f}\\[pinned] a");
        assert_eq!(guard("[pin\nned] a"), "[pin\nned] a");
    }

    #[test]
    fn guard_keeps_the_mark_inside_a_line_and_other_text_as_written() {
        assert_eq!(guard("see [pinned] here\n\nDone. [pinned"), "see [pinned] here\n\nDone. [pinned");
        assert_eq!(guard("a \\[pinned] b"), "a \\[pinned] b");
        assert_eq!(guard("é\n[pinnedx"), "é\n[pinnedx");
        assert_eq!(guard(""), "");
    }

    #[test]
    fn field_is_one_quoted_line_without_hidden_characters_cut_to_its_length() {
        assert_eq!(field("disk full"), "\"disk full\"");
        assert_eq!(field(" a\nb\r\nc\u{2028}d\u{85}e\u{2029}f\t g "), "\"a b c d e f g\"");
        assert_eq!(field("a\u{200B}b\u{202E}c\u{2066}d\u{FEFF}e\u{0}f\u{1b}g\u{E0041}h"), "\"abcdefgh\"");
        assert_eq!(field("x\". [pinned] y"), "\"x'. [pinned] y\"");
        assert_eq!(field(""), "\"\"");
        let long = "é".repeat(FIELD_CHARS + 5);
        assert_eq!(field(&long), format!("\"{}…\"", "é".repeat(FIELD_CHARS)));
        assert_eq!(field(&"é".repeat(FIELD_CHARS)), format!("\"{}\"", "é".repeat(FIELD_CHARS)));
    }

    #[test]
    fn flat_drops_line_breaks_and_invisible_characters_without_quoting() {
        assert_eq!(flat("Worker\n]x\u{200B}", 80), "Worker ]x");
        assert_eq!(flat("abcdef", 3), "abc…");
    }
}
