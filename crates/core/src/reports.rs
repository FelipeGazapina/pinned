//! "Report an issue from the app": the types a report travels in, and the pure functions that turn a draft into a
//! public GitHub issue. Text in, text out, no IO.
//!
//! Everything a report sends out goes through `scrub` first, and `render` scrubs every section itself.

mod scrubber;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What the reporter sends: something that does not work, or a suggestion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Problem,
    Idea,
}

/// What the reporter wrote. A problem uses `happened`, `expected` and `repeat`; an idea uses `happened` as "The idea"
/// and `expected` as "Why it helps".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Draft {
    pub kind: Kind,
    pub title: String,
    #[serde(default)]
    pub happened: String,
    #[serde(default)]
    pub expected: String,
    #[serde(default)]
    pub repeat: String,
    #[serde(default)]
    pub attach: Attach,
    #[serde(default)]
    pub to_issue: Option<u64>,
}

/// What the reporter chose to send along with the text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Attach {
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub errors: Vec<String>,
}

/// The environment section a report carries, after the reporter's choice: none (`false`), the hub's version alone
/// (`true`), or the reporter's app version, system and architecture as the app's dialog shows them, the hub's
/// version after it. Scrubbed like every other part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Environment {
    Chosen(bool),
    Shown(String),
}

impl Default for Environment {
    fn default() -> Environment {
        Environment::Chosen(false)
    }
}

impl Environment {
    /// Whether the reporter chose to send an environment section.
    pub fn chosen(&self) -> bool {
        !matches!(self, Environment::Chosen(false))
    }

    /// What the reporter's app said of itself, empty when it said nothing.
    pub fn text(&self) -> &str {
        match self {
            Environment::Shown(text) => text,
            Environment::Chosen(_) => "",
        }
    }
}

/// What the caller knows must never appear in a public report: the words to replace, by kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Context {
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub names: Vec<String>,
}

/// The issue as it will be sent, with the issues that look alike.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub similar: Vec<Similar>,
    pub left_today: u32,
    pub shown: String,
}

/// An existing issue that looks like the draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Similar {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub comments: u32,
}

/// Where a report is. `Answered`, `Closed`, `Fixed` and `Failed` carry what the state needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReportState {
    Draft,
    Sending,
    Open,
    Answered { at: i64 },
    Closed { reason: String },
    Fixed { version: String },
    Failed { error: String },
}

/// A report the app keeps: a draft, or the issue it became.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub issue: Option<u64>,
    pub url: Option<String>,
    pub followed: bool,
    pub by_agent: bool,
    pub state: ReportState,
    pub draft: Option<Draft>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What the app asks for, by `op`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Ask {
    Status,
    Preview {
        draft: Draft,
        #[serde(default)]
        context: Context,
    },
    Similar {
        title: String,
        /// Words the app knows must not reach the search, merged with the hub's own as a preview's are.
        #[serde(default)]
        context: Context,
    },
    List,
    SaveDraft {
        #[serde(default)]
        id: Option<String>,
        draft: Draft,
    },
    DeleteDraft {
        id: String,
    },
}

/// What an agent may not do with the `reports` action: the app's own preview.
pub fn agent_check_reports(parsed: &crate::surface::Parsed) -> Result<(), String> {
    match parsed {
        crate::surface::Parsed::Hub(crate::wire::HubCall::Reports { ask: Ask::Preview { .. } }) => Err("reports: preview is the app's: the user reads the exact public text there when they approve report_issue".into()),
        _ => Ok(()),
    }
}

/// What an agent may not send with `report_issue`: error lines (only the person's own app attaches them), and the
/// fields only the app's dialog uses.
pub fn agent_check_report_issue(parsed: &crate::surface::Parsed) -> Result<(), String> {
    let crate::surface::Parsed::Hub(crate::wire::HubCall::ReportIssue { draft, shown, draft_id, request_id }) = parsed else { return Ok(()) };
    if !draft.attach.errors.is_empty() {
        return Err("report_issue: an agent attaches no error lines; set attach.environment if the version and system help".into());
    }
    if shown.is_some() || draft_id.is_some() || request_id.is_some() {
        return Err("report_issue: shown, draft_id and request_id are the app's; send only draft".into());
    }
    Ok(())
}

/// Longest title sent, in characters.
const TITLE_CHARS: usize = 120;
/// Longest section of the body, in characters.
const SECTION_CHARS: usize = 4000;
/// Most error lines sent.
const ERROR_LINES: usize = 20;
/// Longest error line sent, in characters.
const ERROR_LINE_CHARS: usize = 300;
/// Most terms in a search query.
const SEARCH_TERMS: usize = 6;

/// Words that never help a search: English and Portuguese stopwords, and the words a placeholder leaves behind.
const SEARCH_STOP: &[&str] = &[
    // English.
    "the", "and", "for", "with", "that", "this", "from", "into", "when", "while", "what", "which", "where", "there", "their", "they", "them", "then", "than", "have",
    "has", "had", "was", "were", "are", "but", "not", "can", "could", "would", "should", "will", "just", "some", "any", "all", "too", "very", "out", "off", "over",
    "under", "after", "before", "about", "again", "also", "only", "its", "does", "did", "doesn", "don", "isn", "wasn", "why", "how", "our", "your", "you", "get",
    // Portuguese.
    "uma", "com", "que", "para", "por", "mais", "como", "mas", "foi", "ser", "sem", "quando", "nos", "nas", "dos", "das", "pelo", "pela", "meu", "minha", "seu", "sua",
    "isso", "esse", "essa", "ainda", "muito", "todo", "toda", "fica", "ficou", "apos", "entre", "sobre", "onde", "pode", "estou", "está", "estao",
    // Placeholder words.
    "folder", "project", "file", "name", "email", "secret",
];

/// Why a text is refused: its markup still changed when scrubbing stopped, so no clean copy of it exists.
pub const UNCLEAN: &str = scrubber::UNCLEAN;

/// Makes `text` safe for a public issue: secrets, e-mail addresses, absolute paths, file names and the words in `ctx`
/// become placeholders, and markup, mentions and issue references are defused. Scrubbing scrubbed text changes
/// nothing. Text that does not settle is refused with `UNCLEAN`, never sent half clean.
pub fn scrub(text: &str, ctx: &Context) -> Result<String, String> {
    scrubber::scrub(text, ctx)
}

/// The title, body and labels of the issue for `draft`. Every part is scrubbed with `scrub(text, ctx)`, here, so a
/// caller cannot forget it. `hub` is the hub's own line of the environment section, after the reporter's.
pub fn render(draft: &Draft, ctx: &Context, hub: Option<&str>, report_ref: &str) -> Result<(String, String, Vec<String>), String> {
    let title = cap(&clean(&draft.title.replace('\n', " "), ctx)?, TITLE_CHARS);
    let mut blocks: Vec<String> = vec![];
    let labels = match draft.kind {
        Kind::Problem => {
            blocks.extend(section("What happened", &draft.happened, ctx)?);
            blocks.extend(section("Expected", &draft.expected, ctx)?);
            blocks.extend(section("How to repeat", &draft.repeat, ctx)?);
            blocks.extend(environment(draft, ctx, hub)?);
            blocks.extend(error_block(&draft.attach.errors, ctx)?);
            ["bug", "from-app", "needs-triage"]
        }
        Kind::Idea => {
            blocks.extend(section("The idea", &draft.happened, ctx)?);
            blocks.extend(section("Why it helps", &draft.expected, ctx)?);
            blocks.extend(environment(draft, ctx, hub)?);
            ["idea", "from-app", "needs-triage"]
        }
    };
    blocks.push(format!("---\n\nSent from the pinned app · report {report_ref}"));
    Ok((title, blocks.join("\n\n"), labels.iter().map(|l| l.to_string()).collect()))
}

/// `text` scrubbed and cut to a title's length, as an agent reads an issue's title.
pub fn public_title(text: &str, ctx: &Context) -> Result<String, String> {
    Ok(cap(&clean(&text.replace('\n', " "), ctx)?, TITLE_CHARS))
}

/// The URL-safe, short name of a report: `r-` and the first 8 hex digits of its UUID, without dashes.
pub fn ref_of(uuid: &str) -> String {
    let hex: String = uuid.chars().filter(|x| *x != '-').take(8).collect::<String>().to_lowercase();
    format!("r-{hex}")
}

/// The hash of an issue as shown: lower-case hex SHA-256 of `title`, a newline and `body`.
pub fn shown(title: &str, body: &str) -> String {
    let digest = Sha256::digest(format!("{title}\n{body}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The words to search existing issues with: the scrubbed title, lower case, without stopwords and placeholder words,
/// the first unique terms of 3 or more characters.
pub fn search_query(title: &str) -> String {
    // Text that does not scrub clean searches for nothing.
    let scrubbed = scrubber::scrub(title, &Context::default()).unwrap_or_default().to_lowercase();
    let mut terms: Vec<&str> = vec![];
    for word in scrubbed.split(|x: char| !x.is_alphanumeric()) {
        if word.chars().count() < 3 || SEARCH_STOP.contains(&word) || terms.contains(&word) {
            continue;
        }
        terms.push(word);
        if terms.len() == SEARCH_TERMS {
            break;
        }
    }
    terms.join(" ")
}

/// The scrubbed text, trimmed.
fn clean(text: &str, ctx: &Context) -> Result<String, String> {
    Ok(scrubber::scrub(text, ctx)?.trim().to_owned())
}

/// The first `max` characters of `text`, without a trailing space. Cut on a character boundary.
fn cap(text: &str, max: usize) -> String {
    text.chars().take(max).collect::<String>().trim_end().to_owned()
}

/// A heading and its text, or nothing when the text is empty.
fn section(heading: &str, text: &str, ctx: &Context) -> Result<Option<String>, String> {
    let body = cap(&clean(text, ctx)?, SECTION_CHARS);
    Ok((!body.is_empty()).then(|| format!("### {heading}\n\n{body}")))
}

/// The environment section, when the reporter chose to send it: the reporter's own line, then the hub's.
fn environment(draft: &Draft, ctx: &Context, hub: Option<&str>) -> Result<Option<String>, String> {
    if !draft.attach.environment.chosen() {
        return Ok(None);
    }
    let reporter = clean(&draft.attach.environment.text().replace('\n', " "), ctx)?;
    let line: Vec<&str> = [reporter.as_str(), hub.unwrap_or_default()].into_iter().filter(|part| !part.trim().is_empty()).collect();
    section("Environment", &line.join(" · "), ctx)
}

/// The last error lines, folded away, when there are any.
fn error_block(errors: &[String], ctx: &Context) -> Result<Option<String>, String> {
    let lines = error_lines(errors, ctx)?;
    Ok((!lines.is_empty()).then(|| format!("<details><summary>Last app error lines (scrubbed)</summary>\n\n```\n{}\n```\n\n</details>", lines.join("\n"))))
}

/// The scrubbed error lines: blank ones dropped, at most `ERROR_LINES` of `ERROR_LINE_CHARS` each. A fence in a line
/// becomes quotes, so the line cannot close the block.
fn error_lines(errors: &[String], ctx: &Context) -> Result<Vec<String>, String> {
    Ok(scrubber::scrub(&errors.join("\n"), ctx)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(ERROR_LINES)
        .map(|line| cap(&line.replace("```", "'''"), ERROR_LINE_CHARS))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOKEN: &str = "ghp_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7r8";

    fn none() -> Context {
        Context::default()
    }

    fn projects(list: &[&str]) -> Context {
        Context { projects: list.iter().map(|x| x.to_string()).collect(), ..Context::default() }
    }

    fn names(list: &[&str]) -> Context {
        Context { names: list.iter().map(|x| x.to_string()).collect(), ..Context::default() }
    }

    fn folders(list: &[&str]) -> Context {
        Context { folders: list.iter().map(|x| x.to_string()).collect(), ..Context::default() }
    }

    fn problem(title: &str) -> Draft {
        Draft { kind: Kind::Problem, title: title.into(), happened: String::new(), expected: String::new(), repeat: String::new(), attach: Attach::default(), to_issue: None }
    }

    /// Scrubs `text` twice and checks that the second round changes nothing.
    fn scrub_twice(text: &str, ctx: &Context) -> String {
        let once = scrub(text, ctx).unwrap();
        assert_eq!(scrub(&once, ctx).unwrap(), once, "second round changed {text:?}");
        once
    }

    #[test]
    fn scrub_replaces_paths_and_file_names_with_folder_and_file() {
        assert_eq!(scrub_twice("/Users/felipe/projetos/loja-web/pedidos.sql", &none()), "[folder]");
        assert_eq!(scrub_twice("pedidos.sql", &none()), "[file]");
    }

    #[test]
    fn scrub_replaces_known_projects_and_names_at_word_boundaries() {
        let ctx = Context { projects: vec!["loja-web".into()], names: vec!["mac-mini".into()], ..Context::default() };
        assert_eq!(scrub_twice("loja-web and mac-mini", &ctx), "[project] and [name]");
        assert_eq!(scrub_twice("loja-web-api", &ctx), "[project]-api");
        assert_eq!(scrub_twice("the acme_db and acme-api", &projects(&["acme"])), "the [project]_db and [project]-api");
        assert_eq!(scrub_twice("acmeweb", &projects(&["acme"])), "acmeweb");
    }

    #[test]
    fn scrub_matches_known_words_longest_first_and_ignores_case() {
        assert_eq!(scrub_twice("Ana Lima and ana", &names(&["Ana", "Ana Lima"])), "[name] and [name]");
        assert_eq!(scrub_twice("Banana and ANA", &names(&["ana"])), "Banana and [name]");
    }

    #[test]
    fn scrub_ignores_known_words_shorter_than_two_characters() {
        assert_eq!(scrub_twice("x y", &names(&["x", " "])), "x y");
    }

    #[test]
    fn scrub_replaces_owner_name_and_clone_urls_with_project() {
        let ctx = projects(&["FelipeGazapina/pinned"]);
        assert_eq!(scrub_twice("git clone https://github.com/FelipeGazapina/pinned.git", &ctx), "git clone https://github.com/[project].git");
        let whole = projects(&["https://github.com/FelipeGazapina/pinned.git"]);
        assert_eq!(scrub_twice("see https://github.com/FelipeGazapina/pinned.git", &whole), "see [project]");
    }

    #[test]
    fn scrub_known_folder_takes_the_path_under_it() {
        assert_eq!(scrub_twice("/Users/ana/proj/a.rs", &folders(&["/Users/ana/proj"])), "[folder]");
    }

    #[test]
    fn scrub_replaces_e_mail_addresses_with_email() {
        assert_eq!(scrub_twice("contact ana.lima+x@example.co.uk now", &none()), "contact [email] now");
        assert_eq!(scrub_twice("ana@example.com.", &none()), "[email].");
    }

    #[test]
    fn scrub_replaces_a_known_name_in_an_address_with_email() {
        assert_eq!(scrub_twice("ana@x.com", &names(&["ana"])), "[email]");
    }

    #[test]
    fn scrub_replaces_token_prefixes_with_secret() {
        assert_eq!(scrub_twice(TOKEN, &none()), "[secret]");
        assert_eq!(scrub_twice("gho_x1 ghs_y2 done", &none()), "[secret] [secret] done");
        assert_eq!(scrub_twice("task-ghp_x1 ghp_", &none()), "task-[secret] ghp_");
    }

    #[test]
    fn scrub_replaces_uuids_with_id() {
        assert_eq!(scrub_twice("7f3a1c2d-9b4e-4f10-8a2c-5d6e7f8a9b0c", &none()), "[id]");
    }

    #[test]
    fn scrub_replaces_tab_ids_with_id() {
        assert_eq!(scrub_twice("tab-8f2a is open", &none()), "[id] is open");
        assert_eq!(scrub_twice("tab-bar stays", &none()), "tab-bar stays");
    }

    #[test]
    fn scrub_defuses_mentions_with_a_zero_width_space() {
        assert_eq!(scrub_twice("@octocat please", &none()), "@\u{200B}octocat please");
    }

    #[test]
    fn scrub_strips_user_info_from_urls_down_to_the_host() {
        assert_eq!(scrub_twice("https://user:pass@host/x", &none()), "https://host");
    }

    #[test]
    fn scrub_strips_the_query_string_from_urls_down_to_the_host() {
        assert_eq!(scrub_twice("https://x.com/a?token=abc123def", &none()), "https://x.com");
    }

    #[test]
    fn scrub_keeps_plain_urls_and_versions_and_prose() {
        assert_eq!(scrub_twice("https://x.com/docs/page", &none()), "https://x.com/docs/page");
        assert_eq!(scrub_twice("https://x.com/home/y", &none()), "https://x.com/home/y", "a URL path is not a folder");
        assert_eq!(scrub_twice("0.18.6", &none()), "0.18.6");
        assert_eq!(scrub_twice("macOS 15.6", &none()), "macOS 15.6");
        assert_eq!(scrub_twice("arm64", &none()), "arm64");
        assert_eq!(scrub_twice("the token is expired", &none()), "the token is expired");
    }

    #[test]
    fn scrub_removes_html_tags_and_images() {
        assert_eq!(scrub_twice("before <img src=x> after ![a](http://x/y.png) end", &none()), "before  after  end");
    }

    #[test]
    fn scrub_collapses_three_or_more_newlines_into_two() {
        assert_eq!(scrub_twice("a\n\n\n\n\nb", &none()), "a\n\nb");
    }

    #[test]
    fn scrub_secret_keys_and_bearer_keep_the_key_and_replace_the_value() {
        assert_eq!(scrub_twice("password=hunter2 api_key: xyz", &none()), "password=[secret] api_key: [secret]");
        assert_eq!(scrub_twice("GITHUB_TOKEN=abc", &none()), "GITHUB_TOKEN=[secret]");
        assert_eq!(scrub_twice("Authorization: Bearer abc.def-123 ok", &none()), "Authorization: Bearer [secret] ok");
        assert_eq!(scrub_twice("name: hunter2", &none()), "name: hunter2");
    }

    #[test]
    fn scrub_long_mixed_runs_are_secrets() {
        assert_eq!(scrub_twice("abcdefghijklmnopqrstuvwxyz123456", &none()), "[secret]");
        assert_eq!(scrub_twice("abcdefghijklmnopqrstuvwxyz12345", &none()), "abcdefghijklmnopqrstuvwxyz12345", "31 characters are not enough");
        assert_eq!(scrub_twice("abcdefghijklmnopqrstuvwxyzABCDEFGHIJ", &none()), "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ", "no digit, not a secret");
    }

    #[test]
    fn scrub_keeps_the_placeholder_on_later_rounds() {
        assert_eq!(scrub_twice("my secret folder", &names(&["folder", "secret"])), "my [name] [name]");
        assert_eq!(scrub_twice("[name] [secret] [folder] [email] [file] [id] [project]", &names(&["name", "secret", "folder", "email", "file", "id", "project"])), "[name] [secret] [folder] [email] [file] [id] [project]");
        assert_eq!(scrub_twice("password=[secret] token [secret]", &none()), "password=[secret] token [secret]");
    }

    #[test]
    fn scrub_takes_the_whole_path_past_a_placeholder() {
        assert_eq!(scrub_twice("/Users/Ana/x", &names(&["Ana"])), "[folder]");
        assert_eq!(scrub_twice("open(\"/home/ana/x\")", &none()), "open(\"[folder]\")");
    }

    #[test]
    fn scrub_is_idempotent_on_mixed_text() {
        let ctx = Context { projects: vec!["acme-core".into()], names: vec!["Ana Lima".into()], folders: vec![], files: vec![] };
        for text in [
            "ghp_AbC123 token abc Bearer xyz",
            "password=hunter2 api_key: xyz /home/ana/x@corp.io/a.rs",
            "Ana Lima at ANA@localhost, see src/app/main.rs and ~/notes.txt",
            "a@b.cc@d.ee token token token: : = ==",
            "@ana @@ @ @acme-core https://u:p@h/x?y=1 ![i](x) <b>bold</b>\n\n\n\n",
        ] {
            scrub_twice(text, &ctx);
        }
    }

    #[test]
    fn scrub_random_strings_never_panic_and_are_a_fixed_point() {
        const PIECES: &[&str] = &[
            "ghp_", "sk-", "Bearer ", "token", "=", ":", " ", "\n", "\n\n\n", "/home/", "/Users/", "~/", "C:\\Users\\", "ana", "Ana", "@", ".", "x.com", "ação", "日", "🚀", "rs",
            "<", ">", "</", "secret", "name", "[", "]", "[name]", "[secret]", "![", "](", "://", "?", "#", "tab-", "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6", "\"", "'", "/", "_", "-",
            "AKIA", "7f3a1c2d-9b4e-4f10-8a2c-5d6e7f8a9b0c", "acme-core", "pedidos.sql", "\u{200B}",
        ];
        let ctx = Context {
            names: vec!["Ana".into(), "ação".into(), "name".into(), "日本".into()],
            projects: vec!["acme-core".into(), "owner/name".into()],
            folders: vec!["/Users/ana".into()],
            files: vec!["pedidos.sql".into()],
        };
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
            // Text that does not settle is refused, which is no clean copy to scrub again.
            if scrub(&text, &ctx).is_ok() {
                scrub_twice(&text, &ctx);
            }
        }
    }

    /// `<` n times, then `b>` n times: removing the innermost `<b>` leaves the next one whole.
    fn nested_tags(n: usize) -> String {
        format!("{}{}", "<".repeat(n), "b>".repeat(n))
    }

    #[test]
    fn scrub_removes_nested_tags_in_one_go_so_the_word_they_split_is_still_found() {
        let ctx = projects(&["loja-web"]);
        for n in [16, 32] {
            let text = format!("lo{}ja-web crashes", nested_tags(n));
            assert_eq!(scrub_twice(&text, &ctx), "[project] crashes", "{n} nested tags");
        }
    }

    #[test]
    fn scrub_removes_nested_images_in_one_go_so_the_word_they_split_is_still_found() {
        let mut images = "![a](x)".to_owned();
        for _ in 0..31 {
            images = format!("![{images}](x)");
        }
        assert_eq!(scrub_twice(&format!("lo{images}ja-web crashes"), &projects(&["loja-web"])), "[project] crashes");
    }

    #[test]
    fn scrub_refuses_markup_that_still_changes_after_the_last_round() {
        // Each level is a tag that an image hides until the round after: one round per level.
        let mut deep = "<b>".to_owned();
        for _ in 0..20 {
            deep = format!("<!{deep}[a](x)b>");
        }
        let text = format!("lo{deep}ja-web");
        assert_eq!(scrub(&text, &projects(&["loja-web"])), Err(UNCLEAN.to_owned()));
        assert_eq!(UNCLEAN, "The report has markup the hub cannot clean. Remove the HTML and try again.");
        let draft = Draft { happened: text, ..problem("Crash") };
        assert_eq!(render(&draft, &projects(&["loja-web"]), None, "r-1"), Err(UNCLEAN.to_owned()));
    }

    #[test]
    fn scrub_finds_a_word_written_with_look_alike_characters() {
        assert_eq!(scrub_twice("ｌｏｊａ-ｗｅｂ broke", &projects(&["loja-web"])), "[project] broke");
    }

    #[test]
    fn scrub_finds_a_word_split_by_zero_width_characters() {
        for invisible in ['\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}'] {
            assert_eq!(scrub_twice(&format!("lo{invisible}ja-web broke"), &projects(&["loja-web"])), "[project] broke", "{invisible:?}");
        }
    }

    #[test]
    fn scrub_finds_a_word_split_by_bidi_controls() {
        for control in ['\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'] {
            assert_eq!(scrub_twice(&format!("lo{control}ja-web broke"), &projects(&["loja-web"])), "[project] broke", "{control:?}");
        }
    }

    #[test]
    fn scrub_takes_a_windows_home_in_any_letter_case() {
        assert_eq!(scrub_twice("at c:\\users\\ana\\proj\\x", &none()), "at [folder]");
        assert_eq!(scrub_twice("at C:\\USERS\\ana\\proj", &none()), "at [folder]");
    }

    #[test]
    fn scrub_takes_paths_under_volumes_mnt_and_srv() {
        assert_eq!(scrub_twice("/Volumes/Backup/clients/x and /mnt/data/y and /srv/site/z", &none()), "[folder] and [folder] and [folder]");
    }

    #[test]
    fn scrub_takes_a_path_written_as_a_file_url() {
        assert_eq!(scrub_twice("open file:///Users/ana/shop/notes and FILE:///home/ana/x", &none()), "open [folder] and [folder]");
        assert_eq!(scrub_twice("file:///Volumes/x/y", &none()), "[folder]");
    }

    #[test]
    fn scrub_takes_a_path_after_an_include_or_library_flag() {
        assert_eq!(scrub_twice("cc -I/home/ana/include -L/Users/ana/lib", &none()), "cc -I[folder] -L[folder]");
    }

    #[test]
    fn scrub_wraps_issue_references_so_github_does_not_link_them() {
        assert_eq!(scrub_twice("see #123 and GH-7.", &none()), "see `#123` and `GH-7`.");
        assert_eq!(scrub_twice("like octo/shop#12 did", &none()), "like `octo/shop#12` did");
        assert_eq!(scrub_twice("#1", &none()), "`#1`");
        assert_eq!(scrub_twice("C#12 and x.com/a#12 and #12a and ### Heading", &none()), "C#12 and x.com/a#12 and #12a and ### Heading");
    }

    #[test]
    fn scrub_keeps_a_placeholder_from_becoming_a_link() {
        let ctx = projects(&["loja-web"]);
        assert_eq!(scrub_twice("[loja-web](https://x.com/a) and loja-web[1]", &ctx), "[[project]](https://x.com/a) and [project] [1]");
        assert_eq!(scrub_twice("loja-web(beta)", &ctx), "[project] (beta)");
        assert_eq!(scrub_twice("x\nloja-web: https://x.com/a", &ctx), "x\n[project] : https://x.com/a");
        assert_eq!(scrub_twice("in loja-web: ok", &ctx), "in [project]: ok", "a colon inside a line is no link definition");
    }

    #[test]
    fn public_title_is_scrubbed_and_cut_to_120_characters() {
        let long = format!("loja-web {}", "x".repeat(200));
        let title = public_title(&long, &projects(&["loja-web"])).unwrap();
        assert_eq!(title.chars().count(), 120);
        assert!(title.starts_with("[project] xxx"), "{title}");
    }

    #[test]
    fn render_puts_the_reporters_environment_first_then_the_hubs() {
        let ctx = names(&["mac-mini"]);
        let shown = Draft { attach: Attach { environment: Environment::Shown("pinned 0.18.6 · macOS 15.6 · arm64 on mac-mini".into()), errors: vec![] }, ..problem("Crash") };
        let (_, body, _) = render(&shown, &ctx, Some("hub 0.18.5"), "r-1").unwrap();
        assert_eq!(body, "### Environment\n\npinned 0.18.6 · macOS 15.6 · arm64 on [name] · hub 0.18.5\n\n---\n\nSent from the pinned app · report r-1");
        let hub_only = Draft { attach: Attach { environment: Environment::Chosen(true), errors: vec![] }, ..problem("Crash") };
        assert_eq!(render(&hub_only, &ctx, Some("hub 0.18.5"), "r-1").unwrap().1, "### Environment\n\nhub 0.18.5\n\n---\n\nSent from the pinned app · report r-1");
        let parsed: Attach = serde_json::from_value(json!({"environment": "pinned 1 · linux"})).unwrap();
        assert_eq!(parsed.environment, Environment::Shown("pinned 1 · linux".into()));
        assert_eq!(serde_json::from_value::<Attach>(json!({"environment": true})).unwrap().environment, Environment::Chosen(true));
    }

    #[test]
    fn render_problem_with_environment_and_errors_gives_the_exact_body() {
        let draft = Draft {
            kind: Kind::Problem,
            title: "Upload stops at 90%".into(),
            happened: "The upload stops at 90% for /Users/felipe/projetos/loja-web/pedidos.sql, sent by ana@example.com.".into(),
            expected: "The file uploads.".into(),
            repeat: "1. Open a big order\n2. Press upload".into(),
            attach: Attach { environment: Environment::Shown("pinned 0.18.6 · macOS 15.6 · arm64".into()), errors: vec!["panic at src/app/main.rs:42".into(), format!("token={TOKEN}")] },
            to_issue: None,
        };
        let (title, body, labels) = render(&draft, &none(), Some("hub 0.18.5"), "r-7f3a1c2d").unwrap();
        assert_eq!(title, "Upload stops at 90%");
        assert_eq!(labels, ["bug", "from-app", "needs-triage"]);
        let expected = concat!(
            "### What happened\n\n",
            "The upload stops at 90% for [folder], sent by [email].\n\n",
            "### Expected\n\n",
            "The file uploads.\n\n",
            "### How to repeat\n\n",
            "1. Open a big order\n2. Press upload\n\n",
            "### Environment\n\n",
            "pinned 0.18.6 · macOS 15.6 · arm64 · hub 0.18.5\n\n",
            "<details><summary>Last app error lines (scrubbed)</summary>\n\n",
            "```\n",
            "panic at [file]:42\n",
            "token=[secret]\n",
            "```\n\n",
            "</details>\n\n",
            "---\n\n",
            "Sent from the pinned app · report r-7f3a1c2d",
        );
        assert_eq!(body, expected);
    }

    #[test]
    fn render_scrubs_every_field_with_the_context() {
        let ctx = Context { projects: vec!["loja-web".into()], names: vec!["mac-mini".into()], ..Context::default() };
        let draft = Draft {
            happened: "loja-web broke".into(),
            expected: "mac-mini works".into(),
            repeat: "open loja-web".into(),
            attach: Attach { environment: Environment::Shown("on mac-mini".into()), errors: vec!["loja-web failed".into()] },
            ..problem("loja-web crash")
        };
        let (title, body, _) = render(&draft, &ctx, Some("hub 1"), "r-1").unwrap();
        assert_eq!(title, "[project] crash");
        assert!(!body.contains("loja-web") && !body.contains("mac-mini"), "{body}");
        assert_eq!(body.matches("[project]").count(), 3, "{body}");
    }

    #[test]
    fn render_idea_gives_its_two_headings_and_the_environment() {
        let draft = Draft {
            kind: Kind::Idea,
            title: "Dark mode for the report".into(),
            happened: "A toggle in settings.".into(),
            expected: "Readers at night.".into(),
            repeat: "not used by an idea".into(),
            attach: Attach { environment: Environment::Chosen(true), errors: vec!["not used by an idea".into()] },
            to_issue: None,
        };
        let (_, body, labels) = render(&draft, &none(), Some("pinned 0.18.6"), "r-7f3a1c2d").unwrap();
        assert_eq!(labels, ["idea", "from-app", "needs-triage"]);
        assert_eq!(
            body,
            "### The idea\n\nA toggle in settings.\n\n### Why it helps\n\nReaders at night.\n\n### Environment\n\npinned 0.18.6\n\n---\n\nSent from the pinned app · report r-7f3a1c2d"
        );
    }

    #[test]
    fn render_leaves_out_an_empty_repeat_and_the_environment_when_not_chosen() {
        let draft = Draft {
            kind: Kind::Problem,
            title: "Crash".into(),
            happened: "It closes.".into(),
            expected: String::new(),
            repeat: "  \n ".into(),
            attach: Attach { environment: Environment::Chosen(false), errors: vec![] },
            to_issue: None,
        };
        let (_, body, _) = render(&draft, &none(), Some("pinned 0.18.6"), "r-7f3a1c2d").unwrap();
        assert_eq!(body, "### What happened\n\nIt closes.\n\n---\n\nSent from the pinned app · report r-7f3a1c2d");
    }

    #[test]
    fn render_labels_follow_the_kind() {
        let (_, _, problem_labels) = render(&problem("Crash"), &none(), None, "r-1").unwrap();
        assert_eq!(problem_labels, ["bug", "from-app", "needs-triage"]);
        let idea = Draft { kind: Kind::Idea, ..problem("Idea") };
        let (_, _, idea_labels) = render(&idea, &none(), None, "r-1").unwrap();
        assert_eq!(idea_labels, ["idea", "from-app", "needs-triage"]);
    }

    #[test]
    fn render_footer_has_the_ref_and_no_at_sign() {
        let draft = Draft { happened: "Something broke.".into(), ..problem("Crash") };
        let (_, body, _) = render(&draft, &none(), None, "r-7f3a1c2d").unwrap();
        assert!(body.ends_with("\n\n---\n\nSent from the pinned app · report r-7f3a1c2d"));
        assert!(!body.contains('@'));
    }

    #[test]
    fn render_caps_the_title_sections_and_error_lines() {
        let errors: Vec<String> = (0..25).map(|_| "e".repeat(400)).collect();
        let draft = Draft {
            happened: "x".repeat(5000),
            attach: Attach { environment: Environment::Chosen(false), errors },
            ..problem(&"t".repeat(130))
        };
        let (title, body, _) = render(&draft, &none(), None, "r-1").unwrap();
        assert_eq!(title.chars().count(), 120);
        let happened = format!("### What happened\n\n{}", "x".repeat(4000));
        assert!(body.starts_with(&happened) && body[happened.len()..].starts_with("\n\n<details>"), "{body:.80}");
        let error_lines: Vec<&str> = body.lines().filter(|l| l.starts_with('e')).collect();
        assert_eq!(error_lines.len(), 20);
        assert!(error_lines.iter().all(|l| l.chars().count() == 300));
    }

    #[test]
    fn render_error_lines_cannot_close_the_block() {
        let draft = Draft { attach: Attach { environment: Environment::Chosen(false), errors: vec!["oops ``` </details> @octocat".into()] }, ..problem("Crash") };
        let (_, body, _) = render(&draft, &none(), None, "r-1").unwrap();
        // The tag and the fence are gone from the line, and the mention is defused.
        assert!(body.contains("oops '''  @\u{200B}octocat\n```"), "{body}");
        assert_eq!(body.matches("```").count(), 2);
        assert_eq!(body.matches("</details>").count(), 1);
    }

    #[test]
    fn shown_is_stable_and_changes_with_one_character_of_the_body() {
        let first = shown("Crash", "It closes.");
        assert_eq!(first, shown("Crash", "It closes."));
        assert_ne!(first, shown("Crash", "It closes!"));
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn search_query_drops_stopwords_and_placeholder_words() {
        assert_eq!(search_query("The export of invoices fails for the customer"), "export invoices fails customer");
        assert_eq!(search_query("Upload of /Users/felipe/a.sql fails with file errors"), "upload fails errors");
        assert_eq!(search_query("one two three four five six seven eight"), "one two three four five six");
        assert_eq!(search_query("go to ab"), "");
    }

    #[test]
    fn ref_of_takes_the_first_eight_hex_digits_without_dashes() {
        assert_eq!(ref_of("7f3a1c2d-9b4e-4f10-8a2c-5d6e7f8a9b0c"), "r-7f3a1c2d");
    }

    #[test]
    fn ask_parses_each_op_and_round_trips() {
        let draft = json!({"kind": "problem", "title": "x"});
        let cases = [
            (json!({"op": "status"}), Ask::Status),
            (json!({"op": "list"}), Ask::List),
            (json!({"op": "similar", "title": "x"}), Ask::Similar { title: "x".into(), context: Context::default() }),
            (json!({"op": "similar", "title": "x", "context": {"names": ["n1"]}}), Ask::Similar { title: "x".into(), context: Context { names: vec!["n1".into()], ..Context::default() } }),
            (json!({"op": "preview", "draft": draft}), Ask::Preview { draft: problem("x"), context: Context::default() }),
            (json!({"op": "save_draft", "draft": draft}), Ask::SaveDraft { id: None, draft: problem("x") }),
            (json!({"op": "save_draft", "id": "d1", "draft": draft}), Ask::SaveDraft { id: Some("d1".into()), draft: problem("x") }),
            (json!({"op": "delete_draft", "id": "d1"}), Ask::DeleteDraft { id: "d1".into() }),
        ];
        for (value, ask) in cases {
            assert_eq!(serde_json::from_value::<Ask>(value.clone()).unwrap(), ask, "{value}");
            let text = serde_json::to_string(&ask).unwrap();
            assert_eq!(serde_json::from_str::<Ask>(&text).unwrap(), ask, "{text}");
            assert_eq!(serde_json::to_value(&ask).unwrap()["op"], value["op"], "{text}");
        }
    }

    #[test]
    fn report_state_answered_is_tagged_by_state() {
        let answered = ReportState::Answered { at: 1 };
        assert_eq!(serde_json::to_value(&answered).unwrap(), json!({"state": "answered", "at": 1}));
        assert_eq!(serde_json::from_value::<ReportState>(json!({"state": "answered", "at": 1})).unwrap(), answered);
        assert_eq!(serde_json::to_value(ReportState::Draft).unwrap(), json!({"state": "draft"}));
    }

    #[test]
    fn an_agent_may_not_preview_attach_errors_or_send_the_apps_fields() {
        use crate::surface::Parsed;
        use crate::wire::HubCall;
        let preview = Parsed::Hub(HubCall::Reports { ask: Ask::Preview { draft: problem("x"), context: Context::default() } });
        assert!(agent_check_reports(&preview).unwrap_err().contains("preview is the app's"));
        assert!(agent_check_reports(&Parsed::Hub(HubCall::Reports { ask: Ask::List })).is_ok());
        let send = |draft: Draft, shown: Option<&str>, draft_id: Option<&str>, request_id: Option<&str>| {
            Parsed::Hub(HubCall::ReportIssue { draft, shown: shown.map(Into::into), draft_id: draft_id.map(Into::into), request_id: request_id.map(Into::into) })
        };
        assert!(agent_check_report_issue(&send(problem("x"), None, None, None)).is_ok());
        let with_errors = Draft { attach: Attach { environment: Environment::Chosen(false), errors: vec!["boom".into()] }, ..problem("x") };
        assert!(agent_check_report_issue(&send(with_errors, None, None, None)).unwrap_err().contains("no error lines"));
        for (shown, draft_id, request_id) in [(Some("h"), None, None), (None, Some("d"), None), (None, None, Some("r"))] {
            assert!(agent_check_report_issue(&send(problem("x"), shown, draft_id, request_id)).is_err());
        }
    }

    #[test]
    fn draft_with_only_kind_and_title_parses() {
        let draft: Draft = serde_json::from_value(json!({"kind": "idea", "title": "x"})).unwrap();
        assert_eq!(draft, Draft { kind: Kind::Idea, title: "x".into(), happened: String::new(), expected: String::new(), repeat: String::new(), attach: Attach::default(), to_issue: None });
    }
}
