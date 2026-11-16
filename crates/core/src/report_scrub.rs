//! Makes text safe for a PUBLIC GitHub issue: secrets, e-mail addresses, absolute paths, file names and the
//! names the caller knows are replaced with fixed placeholders. Pure text in, text out, no dependencies.

/// What the caller knows must never appear in a public report.
#[derive(Debug, Clone, Default)]
pub struct Known {
    /// Project names, file names, people's names, machine names: replaced case-insensitively at word boundaries.
    pub names: Vec<String>,
    /// E-mail addresses known to belong to the reporter (also caught by the generic rule).
    pub emails: Vec<String>,
}

/// Rounds after which the text is returned as it is. One round is enough unless a rule exposes a match for another.
const MAX_ROUNDS: usize = 8;

/// Characters to replace: start, end (exclusive) and the placeholder that stands for them.
type Span = (usize, usize, &'static str);

/// The placeholders. A scan passes over one whole, so a name or an e-mail inside it is never replaced again.
const PLACEHOLDERS: &[&str] = &["<secret>", "<email>", "<path>", "<file>", "<name>"];

/// Prefixes of the tokens that services issue.
const SECRET_PREFIXES: &[&str] = &["ghp_", "gho_", "ghs_", "ghu_", "github_pat_", "sk-", "sk_live_", "xoxb-", "xoxp-", "xapp-", "box_", "AKIA"];

/// Keys whose value is a secret. Matched in lower case anywhere inside the key.
const SECRET_KEYS: &[&str] = &["token", "secret", "password", "apikey", "api_key"];

/// Words that introduce a secret value after a space.
const SECRET_WORDS: &[&str] = &["bearer", "token"];

/// Roots of the absolute paths that belong to one machine.
const PATH_PREFIXES: &[&str] = &["/Users/", "/home/", "/private/", "/var/", "/tmp/", "/opt/", "/root/", "~/"];

/// Extensions that make a token a file name.
const FILE_EXTENSIONS: &[&str] = &[
    "rs", "js", "ts", "tsx", "jsx", "mjs", "py", "go", "rb", "java", "kt", "swift", "json", "toml", "yaml", "yml", "md", "html", "css", "sh", "sql", "lock", "txt", "log", "env",
];

/// Scrubs `text` for a public report. The rules run in order, each replacing what it matches with a placeholder.
/// Scrubbing a scrubbed text changes nothing.
pub fn scrub(text: &str, known: &Known) -> String {
    let names = longest_first(&known.names, 2);
    let emails = longest_first(&known.emails, 1);
    let mut current = text.to_owned();
    for _ in 0..MAX_ROUNDS {
        let next = round(&current, &names, &emails);
        if next == current {
            break;
        }
        current = next;
    }
    current
}

/// One pass of every rule, in order. Each rule scans the output of the one before it.
fn round(text: &str, names: &[String], emails: &[String]) -> String {
    let c: Vec<char> = text.chars().collect();
    // 1. Secrets.
    let c = replace(&c, &prefixed_secret_spans(&c));
    let c = replace(&c, &secret_word_spans(&c));
    let c = replace(&c, &secret_key_spans(&c));
    let c = replace(&c, &long_secret_spans(&c));
    // 2. E-mail addresses.
    let c = replace(&c, &email_spans(&c));
    // 3. Absolute paths: they swallow the e-mails and names inside them, so they come before rules 4 and 5.
    let c = replace(&c, &path_spans(&c));
    // 4. File names.
    let c = replace(&c, &file_spans(&c));
    // 5. Known e-mails, then known names.
    let c = replace(&c, &known_spans(&c, emails, "<email>"));
    let c = replace(&c, &known_spans(&c, names, "<name>"));
    c.into_iter().collect()
}

/// Names and e-mails the caller gave, trimmed and sorted longest first so that the longest match wins.
fn longest_first(values: &[String], min_chars: usize) -> Vec<String> {
    let mut out: Vec<String> = values.iter().map(|v| v.trim().to_owned()).filter(|v| v.chars().count() >= min_chars).collect();
    out.sort_by_key(|v| std::cmp::Reverse(v.chars().count()));
    out
}

/// The text with each span replaced by its placeholder. Spans come in order. An overlap is replaced, never skipped.
fn replace(c: &[char], spans: &[Span]) -> Vec<char> {
    let mut out = Vec::with_capacity(c.len());
    let mut at = 0;
    for &(start, end, placeholder) in spans {
        let start = start.max(at);
        let end = end.min(c.len()).max(start);
        out.extend_from_slice(&c[at..start]);
        out.extend(placeholder.chars());
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

/// Characters of a secret value after a `key:`, `key=` or `token ` word: anything up to a stop, and not a tag or `&`.
fn is_value_char(x: char) -> bool {
    !is_path_stop(x) && !matches!(x, '&' | '<')
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
    if c.get(i) != Some(&'<') {
        return None;
    }
    PLACEHOLDERS.iter().find(|p| occurs_at(c, i, p, false)).map(|p| p.chars().count())
}

/// Rule 1: a token that starts with a known prefix. The prefix must not be glued to a letter or digit before it.
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
            spans.push((i, end, "<secret>"));
            i = end;
        }
    }
    spans
}

/// Rule 1: `Bearer <x>` and `token <x>`. The word stays, the value is replaced.
fn secret_word_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        match secret_word_value(c, i) {
            Some((start, end)) => {
                spans.push((start, end, "<secret>"));
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
    let gap = i + word.len();
    let start = run_end(c, gap, |x| x == ' ' || x == '\t');
    let end = run_end(c, start, is_value_char);
    (start > gap && end > start).then_some((start, end))
}

/// Rule 1: `key=value` and `key: value` where the key names a secret. The key stays, the value is replaced.
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
            spans.push((start, end, "<secret>"));
            done = end;
        }
    }
    spans
}

/// Rule 1: any run of 32 or more characters from base64 and hex that has both a letter and a digit.
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
            spans.push((i, end, "<secret>"));
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
            spans.push((start, end, "<email>"));
            from = end;
        }
    }
    spans
}

/// Rule 3: an absolute path. It runs to the next whitespace or stop character; a placeholder inside it counts whole.
fn path_spans(c: &[char]) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        if path_starts_at(c, i) {
            let end = path_end(c, i);
            spans.push((i, end, "<path>"));
            i = end;
        } else {
            i += 1;
        }
    }
    spans
}

fn path_starts_at(c: &[char], i: usize) -> bool {
    PATH_PREFIXES.iter().any(|p| occurs_at(c, i, p, false))
        || (c.get(i).is_some_and(|x| x.is_ascii_alphabetic()) && c.get(i + 1) == Some(&':') && c.get(i + 2) == Some(&'\\'))
}

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
            spans.push((i, name_end, "<file>"));
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

/// Rule 5: each known name or e-mail, case-insensitively, at word boundaries. The list is longest first.
fn known_spans(c: &[char], needles: &[String], placeholder: &'static str) -> Vec<Span> {
    let mut spans = vec![];
    let mut i = 0;
    while i < c.len() {
        if let Some(len) = placeholder_len(c, i) {
            i += len;
            continue;
        }
        let after_word = i > 0 && is_word_char(c[i - 1]);
        let hit = needles.iter().find(|n| !after_word && occurs_at(c, i, n, true) && !c.get(i + n.chars().count()).is_some_and(|&x| is_word_char(x)));
        match hit {
            Some(needle) => {
                let end = i + needle.chars().count();
                spans.push((i, end, placeholder));
                i = end;
            }
            None => i += 1,
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::{scrub, Known};

    fn names(list: &[&str]) -> Known {
        Known { names: list.iter().map(|n| n.to_string()).collect(), ..Known::default() }
    }

    fn emails(list: &[&str]) -> Known {
        Known { emails: list.iter().map(|e| e.to_string()).collect(), ..Known::default() }
    }

    #[test]
    fn report_scrub_empty_input_stays_empty() {
        assert_eq!(scrub("", &Known::default()), "");
    }

    #[test]
    fn report_scrub_plain_text_is_unchanged() {
        assert_eq!(scrub("the build failed twice, retry in 30s (2/5)", &Known::default()), "the build failed twice, retry in 30s (2/5)");
    }

    #[test]
    fn report_scrub_token_prefixes_are_secrets() {
        assert_eq!(scrub("ghp_AbC123", &Known::default()), "<secret>");
        assert_eq!(scrub("gho_x1 ghs_y2 ghu_z3 github_pat_a1 done", &Known::default()), "<secret> <secret> <secret> <secret> done");
        assert_eq!(scrub("sk-ant-api03 sk_live_9 xoxb-1 xoxp-2 xapp-3 box_4 AKIA5", &Known::default()), "<secret> <secret> <secret> <secret> <secret> <secret> <secret>");
        assert_eq!(scrub("task-ghp_x1 ghp_", &Known::default()), "task-<secret> ghp_");
    }

    #[test]
    fn report_scrub_bearer_and_token_keep_the_word() {
        assert_eq!(scrub("Authorization: Bearer abc.def-123 ok", &Known::default()), "Authorization: Bearer <secret> ok");
        assert_eq!(scrub("token abc123", &Known::default()), "token <secret>");
        assert_eq!(scrub("a bearer  xyz", &Known::default()), "a bearer  <secret>");
    }

    #[test]
    fn report_scrub_secret_keys_keep_the_key() {
        assert_eq!(scrub("password=hunter2 api_key: xyz", &Known::default()), "password=<secret> api_key: <secret>");
        assert_eq!(scrub("GITHUB_TOKEN=abc", &Known::default()), "GITHUB_TOKEN=<secret>");
        assert_eq!(scrub("secret: hunter2", &Known::default()), "secret: <secret>");
        assert_eq!(scrub("name: hunter2", &Known::default()), "name: hunter2");
    }

    #[test]
    fn report_scrub_long_mixed_runs_are_secrets() {
        assert_eq!(scrub("x 0123456789abcdefghijklmnopqrstuvwxyzAB y", &Known::default()), "x <secret> y");
        assert_eq!(scrub("abcdefghijklmnopqrstuvwxyz123456", &Known::default()), "<secret>");
        assert_eq!(scrub("abcdefghijklmnopqrstuvwxyz12345", &Known::default()), "abcdefghijklmnopqrstuvwxyz12345", "31 characters are not enough");
        assert_eq!(scrub("abcdefghijklmnopqrstuvwxyzABCDEFGHIJ", &Known::default()), "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ", "no digit, not a secret");
    }

    #[test]
    fn report_scrub_e_mail_addresses_are_replaced() {
        assert_eq!(scrub("contact ana.lima+x@example.co.uk now", &Known::default()), "contact <email> now");
        assert_eq!(scrub("ação@x.com.", &Known::default()), "<email>.");
        assert_eq!(scrub("user@localhost and a@b.c", &Known::default()), "user@localhost and a@b.c");
    }

    #[test]
    fn report_scrub_absolute_paths_are_replaced() {
        assert_eq!(scrub("see /home/ana/projects/app/main.rs:12:3 now", &Known::default()), "see <path> now");
        assert_eq!(scrub(r#"open("/Users/ana/x")"#, &Known::default()), r#"open("<path>")"#);
        assert_eq!(scrub("/private/var/folders/x y", &Known::default()), "<path> y");
        assert_eq!(scrub("/var/log/app.log", &Known::default()), "<path>");
        assert_eq!(scrub("/tmp/a, /opt/b; /root/c", &Known::default()), "<path>, <path>; <path>");
        assert_eq!(scrub("~/notes.txt", &Known::default()), "<path>");
        assert_eq!(scrub(r"C:\Users\ana\a.txt done", &Known::default()), "<path> done");
    }

    #[test]
    fn report_scrub_file_names_are_replaced() {
        assert_eq!(scrub("edit main.rs and src/app/lib.ts.", &Known::default()), "edit <file> and <file>.");
        assert_eq!(scrub("package.json:12 and README.MD", &Known::default()), "<file>:12 and <file>");
        assert_eq!(scrub("load .env now", &Known::default()), "load <file> now");
        assert_eq!(scrub("image.png and v0.18.5", &Known::default()), "image.png and v0.18.5");
    }

    #[test]
    fn report_scrub_known_names_match_whole_words_case_insensitively() {
        assert_eq!(scrub("Banana and Ana", &names(&["Ana"])), "Banana and <name>");
        assert_eq!(scrub("ANA, ana's Ana_1 Ana-2 Ana.", &names(&["Ana"])), "<name>, <name>'s Ana_1 Ana-2 <name>.");
    }

    #[test]
    fn report_scrub_known_names_are_longest_first() {
        assert_eq!(scrub("Ana Lima and Ana", &names(&["Ana", "Ana Lima"])), "<name> and <name>");
    }

    #[test]
    fn report_scrub_names_shorter_than_two_characters_are_ignored() {
        assert_eq!(scrub("x y", &names(&["x", " "])), "x y");
    }

    #[test]
    fn report_scrub_known_e_mails_are_replaced_case_insensitively() {
        assert_eq!(scrub("mail ANA@localhost now", &emails(&["ana@localhost"])), "mail <email> now");
    }

    #[test]
    fn report_scrub_rules_run_in_order() {
        // A path takes the e-mail and the name inside it, so there is one <path>, not a <path> with holes.
        assert_eq!(scrub("/home/ana/x@corp.io/a.rs", &Known::default()), "<path>");
        assert_eq!(scrub("/Users/Ana/x", &names(&["Ana"])), "<path>");
        assert_eq!(scrub("/home/ana/ghp_abcdef123456", &Known::default()), "<path>");
        // A secret key takes its value before the e-mail rule sees it.
        assert_eq!(scrub("token=ana@x.com", &Known::default()), "token=<secret>");
        // A known e-mail is caught by the generic rule before a known name can cut it up.
        assert_eq!(scrub("ana@x.com", &names(&["ana"])), "<email>");
    }

    #[test]
    fn report_scrub_placeholders_are_not_scrubbed_again() {
        let every_word = names(&["name", "secret", "path", "file", "email"]);
        assert_eq!(scrub("my secret name", &every_word), "my <name> <name>");
        assert_eq!(scrub("<secret> <email> <path> <file> <name>", &every_word), "<secret> <email> <path> <file> <name>");
    }

    #[test]
    fn report_scrub_is_idempotent() {
        let known = Known { names: vec!["Ana Lima".into(), "acme-core".into()], emails: vec!["ana@localhost".into()] };
        for text in [
            "ghp_AbC123 token abc Bearer xyz",
            "password=hunter2 api_key: xyz /home/ana/x@corp.io/a.rs",
            "Ana Lima at ANA@localhost, see src/app/main.rs and ~/notes.txt",
            "a@b.cc@d.ee token token token: : = ==",
        ] {
            let once = scrub(text, &known);
            assert_eq!(scrub(&once, &known), once, "{text:?}");
        }
    }

    #[test]
    fn report_scrub_multibyte_text_is_safe() {
        assert_eq!(scrub("ação 日本 🚀", &names(&["日本"])), "ação <name> 🚀");
        assert_eq!(scrub("ação.rs, ação@x.com, /home/ç/x", &Known::default()), "<file>, <email>, <path>");
        assert_eq!(scrub("josé@x.com ação", &Known::default()), "<email> ação");
    }

    #[test]
    fn report_scrub_realistic_error_log() {
        let log = concat!(
            "thread 'main' panicked at src/app/main.rs:42:7:\n",
            "called `Result::unwrap()` on an `Err` value: request failed\n",
            "  user: ana@example.com\n",
            "  token=ghp_1234567890abcdefghijklmnopqrstuvwxyz\n",
            "  cwd: /home/ana/projects/acme/app\n",
            "  config: /home/ana/projects/acme/app/config.toml\n",
            "  Authorization: Bearer eyJhbGciOiJIUzI1NiJ9\n",
            "  secret: hunter2\n",
            "  at Ana Lima (acme-core)\n",
            "  retry in 30s (attempt 2/5)",
        );
        let known = Known { names: vec!["Ana Lima".into(), "acme-core".into()], ..Known::default() };
        let expected = concat!(
            "thread 'main' panicked at <file>:42:7:\n",
            "called `Result::unwrap()` on an `Err` value: request failed\n",
            "  user: <email>\n",
            "  token=<secret>\n",
            "  cwd: <path>\n",
            "  config: <path>\n",
            "  Authorization: Bearer <secret>\n",
            "  secret: <secret>\n",
            "  at <name> (<name>)\n",
            "  retry in 30s (attempt 2/5)",
        );
        let once = scrub(log, &known);
        assert_eq!(once, expected);
        assert_eq!(scrub(&once, &known), once);
    }

    #[test]
    fn report_scrub_random_text_never_panics_and_is_a_fixed_point() {
        const PIECES: &[&str] = &[
            "ghp_", "sk-", "Bearer ", "token", "=", ":", " ", "\n", "/home/", "~/", "C:\\", "ana", "Ana", "@", ".", "x.com", "ação", "日", "🚀", "rs", "<", ">", "secret", "name",
            "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6", "\"", "'", "/", "_", "-", "AKIA",
        ];
        let known = Known { names: vec!["Ana".into(), "ação".into(), "name".into(), "日本".into()], emails: vec!["ana@x.com".into()] };
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..3000 {
            let len = next() % 40;
            let text: String = (0..len).map(|_| PIECES[next() as usize % PIECES.len()]).collect();
            let once = scrub(&text, &known);
            assert_eq!(scrub(&once, &known), once, "{text:?}");
        }
    }
}
