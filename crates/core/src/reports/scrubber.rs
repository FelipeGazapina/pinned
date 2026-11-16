//! The rules behind `reports::scrub`. Private to `reports`: text in, text out, no IO.
//!
//! The text is NFKC-normalised and loses its invisible characters first. A round then runs the rules in order, each
//! on the output of the one before. Rounds repeat until the text stops changing, so scrubbing scrubbed text changes
//! nothing; text still changing after the last round is refused, never sent half clean.

use super::Context;
use unicode_normalization::UnicodeNormalization;

/// Rounds after which text that still changes is refused. A rule can expose a match for one that ran before it.
const MAX_ROUNDS: usize = 16;

/// Why a text is refused: its markup still changed after the last round.
pub(super) const UNCLEAN: &str = "The report has markup the hub cannot clean. Remove the HTML and try again.";

/// Characters to replace: start, end (exclusive) and what stands for them.
type Span = (usize, usize, String);

/// The placeholders. A rule passes over one whole, so nothing inside one is replaced again.
const PLACEHOLDERS: &[&str] = &["[folder]", "[project]", "[file]", "[name]", "[email]", "[secret]", "[id]"];

/// Prefixes of the tokens that services issue.
const SECRET_PREFIXES: &[&str] = &["ghp_", "gho_", "ghs_", "ghu_", "github_pat_", "sk-", "sk_live_", "xoxb-", "xoxp-", "xapp-", "box_", "AKIA"];

/// Keys whose value is a secret. Matched in lower case anywhere inside the key.
const SECRET_KEYS: &[&str] = &["token", "secret", "password", "apikey", "api_key"];

/// Words that introduce a secret value after a space. `token` only counts when the value looks like a token.
const SECRET_WORDS: &[&str] = &["bearer", "token"];

/// Roots of the absolute paths that belong to one machine.
const PATH_PREFIXES: &[&str] = &["/Users/", "/home/", "/private/", "/var/folders/", "/tmp/", "/opt/", "/root/", "/Volumes/", "/mnt/", "/srv/", "~/", "C:\\Users\\"];

/// Flags a path follows with no space: `-I/home/ana/include`, `-L/Users/ana/lib`.
const PATH_FLAGS: &[&str] = &["-I", "-L"];

/// What may stand just before a path: the start of a word, a quote, an opening bracket, `=` or `:`.
const PATH_LEADS: &[char] = &['"', '\'', '`', '(', '[', '=', ':'];

/// Extensions that make a token a file name.
const FILE_EXTENSIONS: &[&str] = &[
    "rs", "js", "ts", "tsx", "jsx", "mjs", "py", "go", "rb", "java", "kt", "swift", "json", "toml", "yaml", "yml", "md", "html", "css", "sh", "sql", "lock", "txt", "log", "env", "csv",
    "xml", "ini", "cfg", "conf", "pem", "key",
];

/// Scrubs `text` for a public report. The rules run in order, each replacing what it matches with a placeholder.
pub(super) fn scrub(text: &str, ctx: &Context) -> Result<String, String> {
    let known = known_words(ctx);
    let mut current = normalised(text);
    for _ in 0..MAX_ROUNDS {
        let next = round(&current, &known);
        if next == current {
            return Ok(current);
        }
        current = next;
    }
    Err(UNCLEAN.to_owned())
}

/// `text` in NFKC, without zero-width characters and bidi controls, so neither a look-alike nor an invisible
/// character keeps a private word from matching.
fn normalised(text: &str) -> String {
    text.nfkc().filter(|&x| !is_invisible(x)).collect()
}

/// Zero-width characters (U+200B..U+200D, U+2060, U+FEFF) and bidi controls (U+202A..U+202E, U+2066..U+2069).
fn is_invisible(x: char) -> bool {
    matches!(x, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// One pass of every rule, in order. Each rule scans the output of the one before it.
fn round(text: &str, known: &[(String, &'static str)]) -> String {
    let c: Vec<char> = text.chars().collect();
    // E-mail addresses come before the known words, so a known name in the local part cannot leave the domain behind.
    let c = replace(&c, &email_spans(&c));
    // 1. The words the caller knows.
    let c = replace(&c, &known_spans(&c, known));
    // 2. Generic rules: markup, links, ids, secrets, paths and file names.
    let c = until_stable(c, html_spans);
    let c = until_stable(c, image_spans);
    let c = replace(&c, &url_spans(&c));
    let c = replace(&c, &uuid_spans(&c));
    let c = replace(&c, &tab_id_spans(&c));
    let c = replace(&c, &prefixed_secret_spans(&c));
    let c = replace(&c, &secret_word_spans(&c));
    let c = replace(&c, &secret_key_spans(&c));
    let c = replace(&c, &long_secret_spans(&c));
    let c = replace(&c, &path_spans(&c));
    let c = replace(&c, &file_spans(&c));
    // 3. Markdown safety: no pings, no cross-links, no links made of a placeholder, no stray newlines.
    let c = replace(&c, &mention_spans(&c));
    let c = replace(&c, &issue_ref_spans(&c));
    let c = replace(&c, &placeholder_link_spans(&c));
    let c = replace(&c, &newline_spans(&c));
    c.into_iter().collect()
}

/// The words the caller knows, with the placeholder each becomes. Longest first, so the longest match wins.
/// A project also counts as `owner/name`, and a clone URL as `owner/name` too.
fn known_words(ctx: &Context) -> Vec<(String, &'static str)> {
    let folders = ctx.folders.iter().map(|f| (normalised(f.trim()), "[folder]"));
    let projects = ctx.projects.iter().flat_map(|p| project_forms(&normalised(p)).into_iter().map(|f| (f, "[project]")));
    let files = ctx.files.iter().map(|f| (normalised(f.trim()), "[file]"));
    let names = ctx.names.iter().map(|n| (normalised(n.trim()), "[name]"));
    let mut words: Vec<(String, &'static str)> = folders.chain(projects).chain(files).chain(names).filter(|(w, _)| w.chars().count() >= 2).collect();
    words.sort_by_key(|(w, _)| std::cmp::Reverse(w.chars().count()));
    words
}

/// The forms a project shows up in: its name as given, and `owner/name` when the name is a clone URL or `owner/name`.
fn project_forms(project: &str) -> Vec<String> {
    let trimmed = project.trim().trim_end_matches('/');
    let bare = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let mut forms = vec![trimmed.to_owned()];
    let mut parts = bare.rsplit(['/', ':']);
    if let (Some(name), Some(owner)) = (parts.next(), parts.next()) {
        if !name.is_empty() && !owner.is_empty() {
            forms.push(format!("{owner}/{name}"));
        }
    }
    forms
}

/// `c` with `rule`'s spans replaced, again and again until it finds none: removing one tag can join another.
fn until_stable(mut c: Vec<char>, rule: fn(&[char]) -> Vec<Span>) -> Vec<char> {
    loop {
        let spans = rule(&c);
        if spans.is_empty() {
            return c;
        }
        c = replace(&c, &spans);
    }
}

/// The text with each span replaced by its replacement. Spans come in order. An overlap is replaced, never skipped.
fn replace(c: &[char], spans: &[Span]) -> Vec<char> {
    let mut out = Vec::with_capacity(c.len());
    let mut at = 0;
    for (start, end, with) in spans {
        let start = (*start).max(at);
        let end = (*end).min(c.len()).max(start);
        out.extend_from_slice(&c[at..start]);
        out.extend(with.chars());
        at = end;
    }
    out.extend_from_slice(&c[at..]);
    out
}

/// Whether `pat` occurs at `i`. Letter case is ignored when `ci`.
fn occurs_at(c: &[char], i: usize, pat: &str, ci: bool) -> bool {
    pat.chars().enumerate().all(|(k, p)| c.get(i + k).is_some_and(|&x| if ci { fold_case(x) == fold_case(p) } else { x == p }))
}

/// The lower-case form of `x` when it is a single character, else `x`.
fn fold_case(x: char) -> char {
    let mut lower = x.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => x,
    }
}

/// The end of the run of characters that `keep` accepts, starting at `from`.
fn run_end(c: &[char], from: usize, keep: fn(char) -> bool) -> usize {
    let mut end = from;
    while end < c.len() && keep(c[end]) {
        end += 1;
    }
    end
}

/// Letters, digits, `_` and `-`: the characters that make a word, so a name next to one is part of a longer word.
fn is_word_char(x: char) -> bool {
    x.is_alphanumeric() || x == '_' || x == '-'
}

/// The characters of an API token after its prefix, and of a key in `key=value`.
fn is_token_char(x: char) -> bool {
    x.is_ascii_alphanumeric() || x == '_' || x == '-'
}

/// The characters of a string that might be a secret written out in full: base64 and hex with separators.
fn is_long_char(x: char) -> bool {
    x.is_ascii_alphanumeric() || matches!(x, '_' | '-' | '+' | '/' | '=')
}

/// Ends a path, a secret value or a quoted string.
fn is_path_stop(x: char) -> bool {
    x.is_whitespace() || matches!(x, '"' | '\'' | '`' | ')' | ']' | '>' | ',' | ';')
}

/// Characters of a secret value after a `key:`, `key=` or `token ` word: anything up to a stop, and not a tag, `&` or a placeholder.
fn is_value_char(x: char) -> bool {
    !is_path_stop(x) && !matches!(x, '&' | '<' | '[')
}

/// The characters of a file name, relative path included. Letters of any script count.
fn is_file_char(x: char) -> bool {
    x.is_alphanumeric() || matches!(x, '_' | '-' | '.' | '/')
}

fn is_local_char(x: char) -> bool {
    x.is_alphanumeric() || matches!(x, '.' | '_' | '%' | '+' | '-')
}

fn is_domain_char(x: char) -> bool {
    x.is_alphanumeric() || matches!(x, '.' | '-')
}

/// The placeholder that starts at `i`, when there is one: its length in characters.
fn placeholder_len(c: &[char], i: usize) -> Option<usize> {
    if c.get(i) != Some(&'[') {
        return None;
    }
    PLACEHOLDERS.iter().find(|p| occurs_at(c, i, p, false)).map(|p| p.chars().count())
}

/// Rule 1: a known word, case-insensitively, where it stands alone: no letter or digit on either side.
/// A folder also takes the path under it, so `[folder]/a.rs` becomes `[folder]`.
fn known_spans(c: &[char], known: &[(String, &'static str)]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        if let Some(len) = placeholder_len(c, i) {
            i += len;
            continue;
        }
        let word_start = i == 0 || !c[i - 1].is_alphanumeric();
        let hit = known.iter().find(|(w, _)| word_start && occurs_at(c, i, w, true) && !c.get(i + w.chars().count()).is_some_and(|x| x.is_alphanumeric()));
        match hit {
            Some((w, placeholder)) => {
                let mut end = i + w.chars().count();
                if *placeholder == "[folder]" && matches!(c.get(end), Some('/' | '\\')) {
                    end = path_end(c, end);
                }
                spans.push((i, end, placeholder.to_string()));
                i = end;
            }
            None => i += 1,
        }
    }
    spans
}

/// Rule 2: an HTML tag such as `<img src=x>` or `</details>`: `<`, an optional `/`, a tag name, then up to `>`.
fn html_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        if c[i] != '<' {
            i += 1;
            continue;
        }
        let mut name = i + 1;
        if c.get(name) == Some(&'/') {
            name += 1;
        }
        if !c.get(name).is_some_and(|x| x.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        let mut close = run_end(c, name, |x| x.is_ascii_alphanumeric());
        while close < c.len() && c[close] != '<' && c[close] != '>' {
            close += 1;
        }
        if c.get(close) == Some(&'>') {
            spans.push((i, close + 1, String::new()));
            i = close + 1;
        } else {
            i += 1;
        }
    }
    spans
}

/// Rule 2: an image, `![alt](url)`, is removed whole.
fn image_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        if occurs_at(c, i, "![", false) {
            if let Some(end) = image_end(c, i + 1) {
                spans.push((i, end, String::new()));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    spans
}

/// The end of `[alt](url)` when the `[` at `open` starts one.
fn image_end(c: &[char], open: usize) -> Option<usize> {
    let close = (open + 1..c.len()).find(|&k| matches!(c[k], ']' | '[' | '\n'))?;
    if c[close] != ']' || c.get(close + 1) != Some(&'(') {
        return None;
    }
    let paren = (close + 2..c.len()).find(|&k| matches!(c[k], ')' | '\n'))?;
    (c[paren] == ')').then_some(paren + 1)
}

/// Rule 2: `scheme://user:pass@host/path?query` becomes `scheme://host` when it has user info or a query string.
fn url_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i + 3 <= c.len() {
        if !occurs_at(c, i, "://", false) {
            i += 1;
            continue;
        }
        let mut scheme = i;
        while scheme > 0 && (c[scheme - 1].is_ascii_alphanumeric() || matches!(c[scheme - 1], '+' | '-' | '.')) {
            scheme -= 1;
        }
        if scheme == i || !c[scheme].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let authority = i + 3;
        let end = run_end(c, authority, |x| !is_path_stop(x) && x != '<');
        let run = &c[authority..end];
        let auth = &run[..run.iter().position(|&x| matches!(x, '/' | '?' | '#')).unwrap_or(run.len())];
        let has_query = run.contains(&'?');
        if auth.contains(&'@') || has_query {
            let host_start = auth.iter().rposition(|&x| x == '@').map_or(0, |p| p + 1);
            let scheme_text: String = c[scheme..i].iter().collect();
            let host: String = auth[host_start..].iter().collect();
            spans.push((scheme, end, format!("{scheme_text}://{host}")));
        }
        i = end;
    }
    spans
}

/// Rule 3: a UUID, dashes included, is an id.
fn uuid_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i + 36 <= c.len() {
        let fits = c[i..i + 36].iter().enumerate().all(|(k, &x)| if matches!(k, 8 | 13 | 18 | 23) { x == '-' } else { x.is_ascii_hexdigit() });
        let bounded = (i == 0 || !c[i - 1].is_alphanumeric()) && !c.get(i + 36).is_some_and(|x| x.is_alphanumeric());
        if fits && bounded {
            spans.push((i, i + 36, "[id]".to_owned()));
            i += 36;
        } else {
            i += 1;
        }
    }
    spans
}

/// Rule 3: `tab-<id>` is an id, when the id is 4 or more characters with a digit in it.
fn tab_id_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        if occurs_at(c, i, "tab-", true) && (i == 0 || !c[i - 1].is_alphanumeric()) {
            let body = i + 4;
            let end = run_end(c, body, |x| x.is_ascii_alphanumeric());
            if end - body >= 4 && c[body..end].iter().any(|x| x.is_ascii_digit()) {
                spans.push((i, end, "[id]".to_owned()));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    spans
}

/// Rule 3: a token that starts with a known prefix. The prefix must not be glued to a letter or digit before it.
fn prefixed_secret_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        let glued = i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_');
        let Some(prefix) = SECRET_PREFIXES.iter().find(|p| !glued && occurs_at(c, i, p, false)) else {
            i += 1;
            continue;
        };
        let body = i + prefix.chars().count();
        let end = run_end(c, body, is_token_char);
        if end == body {
            i += 1;
        } else {
            spans.push((i, end, "[secret]".to_owned()));
            i = end;
        }
    }
    spans
}

/// Rule 3: `Bearer <x>`, and `token <x>` when `<x>` looks like a token. The word stays, the value is replaced.
fn secret_word_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        match secret_word_value(c, i) {
            Some((start, end)) => {
                spans.push((start, end, "[secret]".to_owned()));
                i = end;
            }
            None => i += 1,
        }
    }
    spans
}

/// The value after a secret word at `i` and at least one space or tab: its start and end.
fn secret_word_value(c: &[char], i: usize) -> Option<(usize, usize)> {
    if i > 0 && is_word_char(c[i - 1]) {
        return None;
    }
    let word = SECRET_WORDS.iter().find(|w| occurs_at(c, i, w, true))?;
    let gap = i + word.chars().count();
    let start = run_end(c, gap, |x| x == ' ' || x == '\t');
    let end = run_end(c, start, is_value_char);
    if start == gap || end == start {
        return None;
    }
    // `token` is also an ordinary word: "the token is expired" keeps its text. A token has 16+ characters and a digit.
    let token_like = end - start >= 16 && c[start..end].iter().any(|x| x.is_ascii_digit());
    (*word == "bearer" || token_like).then_some((start, end))
}

/// Rule 3: `key=value` and `key: value` where the key names a secret. The key stays, the value is replaced.
fn secret_key_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut done = 0;
    for (sep, &x) in c.iter().enumerate() {
        if sep < done || (x != '=' && x != ':') {
            continue;
        }
        let mut key_start = sep;
        while key_start > 0 && is_token_char(c[key_start - 1]) {
            key_start -= 1;
        }
        let key = c[key_start..sep].iter().collect::<String>().to_ascii_lowercase();
        if !SECRET_KEYS.iter().any(|k| key.contains(*k)) {
            continue;
        }
        let start = run_end(c, sep + 1, |y| y == ' ' || y == '\t');
        let end = run_end(c, start, is_value_char);
        if end > start {
            spans.push((start, end, "[secret]".to_owned()));
            done = end;
        }
    }
    spans
}

/// Rule 3: any run of 32 or more characters from base64 and hex that has both a letter and a digit.
fn long_secret_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        if !is_long_char(c[i]) {
            i += 1;
            continue;
        }
        let end = run_end(c, i, is_long_char);
        let run = &c[i..end];
        if run.len() >= 32 && run.iter().any(|x| x.is_ascii_alphabetic()) && run.iter().any(|x| x.is_ascii_digit()) {
            spans.push((i, end, "[secret]".to_owned()));
        }
        i = end;
    }
    spans
}

/// Rule 2: `local@domain.tld`. The local part is the run of address characters before the `@`, the domain the run after it.
fn email_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut from = 0;
    for (at, &x) in c.iter().enumerate() {
        if x != '@' || at < from {
            continue;
        }
        let mut start = at;
        while start > from && is_local_char(c[start - 1]) {
            start -= 1;
        }
        let mut end = at + 1;
        while end < c.len() && is_domain_char(c[end]) {
            end += 1;
        }
        while end > at + 1 && matches!(c[end - 1], '.' | '-') {
            end -= 1;
        }
        let domain = &c[at + 1..end];
        let tld_ok = domain
            .iter()
            .rposition(|&x| x == '.')
            .map(|dot| &domain[dot + 1..])
            .is_some_and(|tld| tld.len() >= 2 && tld.iter().all(|x| x.is_alphabetic()));
        if start < at && tld_ok {
            spans.push((start, end, "[email]".to_owned()));
            from = end;
        }
    }
    spans
}

/// Rule 4: an absolute path, also as a `file://` URL. It runs to the next whitespace or stop character; a
/// placeholder inside it counts whole.
fn path_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        let url = occurs_at(c, i, "file://", true) && is_led(c, i) && has_root(c, i + 7);
        if url || path_starts_at(c, i) {
            let end = path_end(c, i);
            spans.push((i, end, "[folder]".to_owned()));
            i = end;
        } else {
            i += 1;
        }
    }
    spans
}

/// Whether a path of this machine starts at `i`: a known root, after the start of the text, a lead character or a
/// flag such as `-I`.
fn path_starts_at(c: &[char], i: usize) -> bool {
    let flagged = i >= 2 && PATH_FLAGS.iter().any(|f| occurs_at(c, i - 2, f, false)) && is_led(c, i - 2);
    (is_led(c, i) || flagged) && has_root(c, i)
}

/// Whether `i` is the start of the text, or follows whitespace or a lead character.
fn is_led(c: &[char], i: usize) -> bool {
    i == 0 || c[i - 1].is_whitespace() || PATH_LEADS.contains(&c[i - 1])
}

/// Whether a known root starts at `i`, in any letter case.
fn has_root(c: &[char], i: usize) -> bool {
    PATH_PREFIXES.iter().any(|p| occurs_at(c, i, p, true))
}

/// The end of the path that starts at `from`: the next stop character, with a placeholder passed over whole.
fn path_end(c: &[char], from: usize) -> usize {
    let mut end = from;
    while end < c.len() {
        if let Some(len) = placeholder_len(c, end) {
            end += len;
        } else if is_path_stop(c[end]) {
            break;
        } else {
            end += 1;
        }
    }
    end
}

/// Rule 4: a token `name.ext` with a known extension, or a relative path whose last part is one.
fn file_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        if !is_file_char(c[i]) {
            i += 1;
            continue;
        }
        let end = run_end(c, i, is_file_char);
        // A dot that ends a sentence is not part of the name.
        let mut name_end = end;
        while name_end > i && c[name_end - 1] == '.' {
            name_end -= 1;
        }
        if is_file_name(&c[i..name_end]) {
            spans.push((i, name_end, "[file]".to_owned()));
        }
        i = end;
    }
    spans
}

/// Whether `token` ends in `.ext` for a known extension. A bare `.env` counts too.
fn is_file_name(token: &[char]) -> bool {
    let segment = match token.iter().rposition(|&x| x == '/') {
        Some(slash) => &token[slash + 1..],
        None => token,
    };
    let Some(dot) = segment.iter().rposition(|&x| x == '.') else {
        return false;
    };
    let stem = &segment[..dot];
    let ext = segment[dot + 1..].iter().collect::<String>().to_ascii_lowercase();
    FILE_EXTENSIONS.contains(&ext.as_str()) && (!stem.is_empty() || ext == "env")
}

/// Rule 5: an `@` that starts a name gets a zero-width space after it, so GitHub does not ping anyone.
/// An `@` inside a word, as in an address, is left alone.
fn mention_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    for (i, &x) in c.iter().enumerate() {
        if x == '@' && (i == 0 || !c[i - 1].is_alphanumeric()) && c.get(i + 1).is_some_and(|y| y.is_alphanumeric()) {
            spans.push((i + 1, i + 1, "\u{200B}".to_owned()));
        }
    }
    spans
}

/// Rule 5: an issue reference GitHub would link, `#123`, `GH-123` or `owner/repo#123`, is wrapped in backticks, so
/// it shows as written and links nowhere. One already in backticks is left alone.
fn issue_ref_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        let digits = if c[i] == '#' {
            i + 1
        } else if occurs_at(c, i, "GH-", true) && (i == 0 || !is_word_char(c[i - 1])) {
            i + 3
        } else {
            i += 1;
            continue;
        };
        let end = run_end(c, digits, |x| x.is_ascii_digit());
        if end == digits || c.get(end).is_some_and(|&x| is_word_char(x)) {
            i += 1;
            continue;
        }
        let start = if c[i] == '#' { repo_start(c, i) } else { i };
        let glued = start > 0 && (is_word_char(c[start - 1]) || matches!(c[start - 1], '`' | '/' | '#' | '.'));
        if !glued {
            let text: String = c[start..end].iter().collect();
            spans.push((start, end, format!("`{text}`")));
        }
        i = end;
    }
    spans
}

/// Where `owner/repo` starts right before the `#` at `hash`, or `hash` when there is none.
fn repo_start(c: &[char], hash: usize) -> usize {
    let is_repo_char = |x: char| x.is_ascii_alphanumeric() || matches!(x, '-' | '_' | '.');
    let mut name = hash;
    while name > 0 && is_repo_char(c[name - 1]) {
        name -= 1;
    }
    if name == hash || name == 0 || c[name - 1] != '/' {
        return hash;
    }
    let mut owner = name - 1;
    while owner > 0 && (c[owner - 1].is_ascii_alphanumeric() || c[owner - 1] == '-') {
        owner -= 1;
    }
    if owner == name - 1 {
        return hash;
    }
    owner
}

/// Rule 5: a placeholder right before `(` or `[`, or one that starts a line before `:`, would make a Markdown link or
/// link definition: a space goes after it.
fn placeholder_link_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        let Some(len) = placeholder_len(c, i) else {
            i += 1;
            continue;
        };
        let end = i + len;
        let line_start = c[..i].iter().rposition(|&x| x == '\n').map_or(0, |p| p + 1);
        let starts_line = c[line_start..i].iter().all(|&x| x == ' ');
        if matches!(c.get(end), Some('(' | '[')) || (starts_line && c.get(end) == Some(&':')) {
            spans.push((end, end, " ".to_owned()));
        }
        i = end;
    }
    spans
}

/// Rule 5: three or more newlines in a row become two.
fn newline_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        if c[i] != '\n' {
            i += 1;
            continue;
        }
        let end = run_end(c, i, |x| x == '\n');
        if end - i > 2 {
            spans.push((i + 2, end, String::new()));
        }
        i = end;
    }
    spans
}
