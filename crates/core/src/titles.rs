//! A new title for a conversation from a short `claude -p`, as T3 Code regenerates thread titles.

use std::path::Path;
use std::process::{Command, Stdio};

/// T3 Code's regenerate prompt, its editorial rules unchanged.
const RULES: &str = "Determine the title in this order:
1. Read the USER messages first. Identify the latest explicit durable goal. The original subject remains the subject until the user clearly changes what the thread is about.
2. Use ASSISTANT messages to resolve vague links, unnamed code, and discovered product nouns. Do not promote one assistant finding into the thread subject unless the user adopts it as a new goal.
3. Compare that subject with the previous title. Preserve accurate scope words, especially when earlier content is truncated. Replace the previous title when it is generic, artifact-based, a completion update, or contradicted by the thread.
4. Title the durable subject and desired outcome, not the current workflow state.

Editorial rules:
- 3-8 words, fewer than 40 characters.
- Use a compact noun phrase or clear action phrase.
- Preserve the umbrella subject when later messages focus on one finding, provider, platform, or implementation detail.
- Ignore deliverables and operations such as mocks, plans, HTML, branches, PRs, tests, CI, commits, merging, and monitoring unless they are the actual topic.
- Models, subagents, tools, output formats, and monitoring instructions do not belong in the title unless they are themselves the topic.
- Do not claim the work is complete.
- Do not copy and truncate a thread message.
- Avoid project names, PR numbers, quotes, labels, filler, and trailing punctuation.
- Keep the previous title unchanged if it is already accurate. Otherwise return a meaningfully improved title, not a cosmetic paraphrase.";

fn prompt(previous: &str, thread: &str) -> String {
    format!(
        "Regenerate the title for an existing coding thread so the user can recognize it weeks later.\nThe previous title was {}.\nReturn only JSON with the key title.\n\n{RULES}\n\nThe thread:\n{thread}",
        serde_json::to_string(previous).unwrap_or_default()
    )
}

/// Runs `claude` without tools, hooks or a saved session, since only its answer matters.
pub fn regenerate(claude: &Path, cwd: &Path, path: Option<&str>, previous: &str, thread: &str) -> Result<String, String> {
    let mut command = Command::new(claude);
    command
        .args(["-p", "--no-session-persistence", "--model", "haiku", "--tools", "", "--settings", r#"{"disableAllHooks":true}"#])
        .arg(prompt(previous, thread))
        .current_dir(cwd)
        .stdin(Stdio::null());
    for key in crate::machine::INHERITED_SESSION_ENV {
        command.env_remove(key);
    }
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let out = command.output().map_err(|e| format!("couldn't run claude: {e}"))?;
    if !out.status.success() {
        return Err(format!("claude: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    title_of(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| "claude gave no title".to_owned())
}

/// The title from claude's answer: the JSON it was asked for, or else its first line.
fn title_of(answer: &str) -> Option<String> {
    let json = answer.find('{').zip(answer.rfind('}')).and_then(|(a, b)| serde_json::from_str::<serde_json::Value>(&answer[a..=b]).ok());
    let raw = match json.as_ref().and_then(|v| v["title"].as_str()) {
        Some(title) => title.to_owned(),
        None => answer.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("```"))?.to_owned(),
    };
    let title: String = raw.trim().trim_matches(|c: char| c == '"' || c == '\'' || c == '`').trim_end_matches(['.', '!', ':']).trim().chars().take(80).collect();
    (!title.is_empty()).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_comes_from_the_json_or_the_first_line() {
        assert_eq!(title_of(r#"{"title":"Fix Sidebar PR Badges"}"#).as_deref(), Some("Fix Sidebar PR Badges"));
        assert_eq!(title_of("```json\n{\"title\": \"Settle Merged Threads.\"}\n```").as_deref(), Some("Settle Merged Threads"));
        assert_eq!(title_of("\n\"Rename the Sidebar\"\nmore").as_deref(), Some("Rename the Sidebar"));
        assert_eq!(title_of("  \n"), None);
    }

    #[test]
    fn the_prompt_quotes_the_previous_title_and_carries_the_thread() {
        let p = prompt("Old \"one\"", "USER: make it faster");
        assert!(p.contains(r#"The previous title was "Old \"one\"""#), "{p}");
        assert!(p.ends_with("The thread:\nUSER: make it faster"));
    }
}
