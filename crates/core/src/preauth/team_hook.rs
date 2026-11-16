//! `pinned team-guard`: Claude Code's PreToolUse hook in a team's box. A never-list hit (a deploy,
//! a release, a push to main ...) is the kind of step only people may approve, and a team's bot has
//! no user at its terminal. So the hook asks the team (`TeamOp::AskPeople`, `BeforeIrreversible`)
//! and denies the tool call until the answer comes back as a message. It fails closed: when the
//! hub can't be asked, the step is denied.
//!
//! Outside a team box (`PINNED_TEAM_BOX` not `1`) the hook does nothing, and no settings file
//! registers it: the never list is still only a pure function there.

use super::never::{never, Ask, AskCtx};
use crate::agent::Message;
use crate::team_call;
use crate::teams::Situation;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const SUBCOMMAND: &str = "team-guard";

/// What a session reads when the hub could not be asked.
pub const COULD_NOT_ASK: &str = "Could not ask the team; do not do this step.";
/// The choices every step's question offers.
pub const GO_AHEAD: &str = "Go ahead";
pub const DONT: &str = "Don't";

const QUESTION_CHARS: usize = 300;

/// What the hook knows besides the payload.
#[derive(Debug, Clone, Default)]
pub struct HookEnv {
    /// `PINNED_TEAM_BOX` says this is a team's box.
    pub team_box: bool,
    /// `CLAUDE_PROJECT_DIR`: where the session started, which a `cd` does not move.
    pub project_dir: Option<PathBuf>,
    pub home: PathBuf,
}

impl HookEnv {
    fn from_process() -> HookEnv {
        HookEnv {
            team_box: team_call::in_team_box(),
            project_dir: std::env::var("CLAUDE_PROJECT_DIR").ok().filter(|v| !v.is_empty()).map(PathBuf::from),
            home: crate::machine::home(),
        }
    }
}

/// Runs inside the hook process. Prints a deny, or nothing so Claude Code's own permissions decide.
pub fn run() {
    let payload = crate::hooks::hook_payload();
    if let Some(output) = decide(&payload, &HookEnv::from_process(), crate::hooks::ask) {
        println!("{output}");
    }
}

/// The hook's stdout for one PreToolUse payload: `None` when the call may go on (not a team box, no
/// never-list hit, or a payload that is no tool call), else a deny. `ask` sends the node a message
/// and returns the hub's answer.
pub fn decide(payload: &str, env: &HookEnv, ask: impl FnOnce(Message) -> Result<String, String>) -> Option<String> {
    if !env.team_box {
        return None;
    }
    let hook: Value = serde_json::from_str(payload).ok()?;
    let input = &hook["tool_input"];
    let tool_ask = Ask::from_tool(hook["tool_name"].as_str()?, input)?;
    let cwd = PathBuf::from(hook["cwd"].as_str()?);
    let start = env.project_dir.clone().unwrap_or_else(|| cwd.clone());
    let worktree = repo_top(&start).unwrap_or_else(|| start.clone());
    let worktree = std::fs::canonicalize(&worktree).unwrap_or(worktree);
    let current_branch = repo_top(&cwd).and_then(|top| branch(&top));
    let default_branch = repo_top(&cwd).and_then(|top| default_branch(&top)).unwrap_or_else(|| "main".into());
    let ctx = AskCtx { worktree: &worktree, scratch: None, home: &env.home, cwd: &cwd, default_branch: &default_branch, current_branch: current_branch.as_deref() };
    let hit = never(&ctx, &tool_ask)?;
    let question = format!("{}: {}", hit.item.label(), step(&tool_ask, input));
    let reason = match ask(Message::AskPeople { situation: Situation::BeforeIrreversible, question, choices: vec![GO_AHEAD.into(), DONT.into()] })
        .and_then(|reply| team_call::parse_asked(&reply))
    {
        Ok(asked) => format!("Asked the team before this step (call {}). End your turn and wait for the answer; do it only if the answer says go ahead.", asked.id),
        Err(_) => COULD_NOT_ASK.to_owned(),
    };
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } }).to_string())
}

/// The step on one line, with tokens and secret-named assignments hidden, cut to a length a person
/// reads in a notification.
fn step(ask: &Ask, input: &Value) -> String {
    let text = match ask {
        Ask::Bash { command } => (*command).to_owned(),
        Ask::Edit { path } | Ask::Read { path } => path.display().to_string(),
        Ask::WebFetch { url } => (*url).to_owned(),
        Ask::Mcp { name, .. } => format!("{name} {input}"),
    };
    scrub(&text)
}

/// `text` on one line with keys and tokens hidden, and the value of any `NAME=value` whose name says
/// secret.
fn scrub(text: &str) -> String {
    let masked = crate::surface::mask_secrets(text);
    let line = masked
        .split_whitespace()
        .map(|word| match word.split_once('=') {
            Some((name, value)) if !value.is_empty() && secret_name(name) => format!("{name}=[hidden]"),
            _ => word.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    match line.char_indices().nth(QUESTION_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

fn secret_name(name: &str) -> bool {
    let name = name.trim_start_matches('-').to_ascii_uppercase();
    ["TOKEN", "SECRET", "PASSWORD", "PASSWD", "KEY", "CREDENTIAL", "AUTH"].iter().any(|word| name.contains(word))
}

/// The folder holding `.git` at or above `start`.
fn repo_top(start: &Path) -> Option<PathBuf> {
    start.ancestors().find(|dir| dir.join(".git").exists()).map(Path::to_owned)
}

/// The gitdir of a checkout, following a worktree's `.git` file.
fn gitdir(top: &Path) -> Option<PathBuf> {
    let dot = top.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    Some(top.join(text.trim().strip_prefix("gitdir:")?.trim()))
}

fn branch(top: &Path) -> Option<String> {
    let head = std::fs::read_to_string(gitdir(top)?.join("HEAD")).ok()?;
    head.trim().strip_prefix("ref: refs/heads/").map(str::to_owned)
}

fn default_branch(top: &Path) -> Option<String> {
    let gitdir = gitdir(top)?;
    let common = std::fs::read_to_string(gitdir.join("commondir")).map_or_else(|_| gitdir.clone(), |c| gitdir.join(c.trim()));
    let head = std::fs::read_to_string(common.join("refs/remotes/origin/HEAD")).ok()?;
    head.trim().strip_prefix("ref: refs/remotes/origin/").map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn env(team_box: bool) -> HookEnv {
        HookEnv { team_box, project_dir: Some(PathBuf::from("/work/site")), home: PathBuf::from("/home/box") }
    }

    fn payload(tool: &str, input: Value) -> String {
        json!({ "tool_name": tool, "tool_input": input, "cwd": "/work/site", "permission_mode": "bypassPermissions" }).to_string()
    }

    fn decision(payload: &str, team_box: bool, reply: Result<&str, &str>) -> (Option<Value>, Option<Message>) {
        let seen = RefCell::new(None);
        let out = decide(payload, &env(team_box), |m| {
            *seen.borrow_mut() = Some(m);
            reply.map(str::to_owned).map_err(str::to_owned)
        });
        (out.map(|o| serde_json::from_str(&o).unwrap()), seen.into_inner())
    }

    fn denied(out: &Value) -> &str {
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
        out["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap()
    }

    #[test]
    fn a_deploy_in_a_team_box_asks_the_team_and_is_denied() {
        let deploy = payload("Bash", json!({ "command": "vercel deploy --prod --token vca_abc" }));
        let (out, message) = decision(&deploy, true, Ok(r#"{"id":"k7","asked":["the owner"]}"#));
        assert_eq!(denied(&out.unwrap()), "Asked the team before this step (call k7). End your turn and wait for the answer; do it only if the answer says go ahead.");
        let Some(Message::AskPeople { situation, question, choices }) = message else { panic!("no ask") };
        assert_eq!(situation, Situation::BeforeIrreversible);
        assert_eq!(choices, ["Go ahead", "Don't"]);
        assert!(question.starts_with("deploy: vercel deploy --prod"), "{question}");
        assert!(!question.contains('\n'));
    }

    #[test]
    fn the_question_names_the_matched_item_and_hides_tokens() {
        let release = payload("Bash", json!({ "command": "GH_TOKEN=ghp_0123456789abcdefghij0123 gh release create v1.0.0 --notes 'x'" }));
        let (out, message) = decision(&release, true, Ok(r#"{"id":"k1"}"#));
        assert!(out.is_some());
        let Some(Message::AskPeople { question, .. }) = message else { panic!("no ask") };
        assert!(question.starts_with("release: "), "{question}");
        assert!(question.contains("gh release create v1.0.0"), "{question}");
        assert!(!question.contains("ghp_") && !question.contains("0123456789"), "{question}");
        assert_eq!(scrub("curl -H 'x' API_KEY=abc123 --password=hunter2 sk-0123456789abcdefghij0123"), "curl -H 'x' API_KEY=[hidden] --password=[hidden] [hidden]");
        assert_eq!(scrub(&"a ".repeat(400)).chars().count(), QUESTION_CHARS + 1);
    }

    #[test]
    fn an_unreachable_hub_denies_the_step() {
        let deploy = payload("Bash", json!({ "command": "vercel deploy --prod" }));
        for reply in [Err("not connected to the hub"), Err("the hub did not answer within 60 seconds"), Ok("not a call")] {
            let (out, message) = decision(&deploy, true, reply);
            assert_eq!(denied(&out.unwrap()), "Could not ask the team; do not do this step.");
            assert!(message.is_some());
        }
    }

    #[test]
    fn outside_a_team_box_the_hook_does_nothing_and_asks_nobody() {
        // The old behavior: no PreToolUse hook, so the never list only informs the prompt.
        let deploy = payload("Bash", json!({ "command": "vercel deploy --prod" }));
        let (out, message) = decision(&deploy, false, Ok(r#"{"id":"k1"}"#));
        assert!(out.is_none() && message.is_none());
    }

    #[test]
    fn a_step_off_the_never_list_runs_without_asking() {
        for call in [payload("Bash", json!({ "command": "cargo test" })), payload("Read", json!({ "file_path": "/work/site/README.md" })), payload("TodoWrite", json!({})), "not json".into()] {
            let (out, message) = decision(&call, true, Ok(r#"{"id":"k1"}"#));
            assert!(out.is_none() && message.is_none(), "{call}");
        }
    }

    #[test]
    fn only_a_team_boxs_settings_register_the_hook() {
        let exe = Path::new("/usr/local/bin/pinned");
        let plain = crate::hooks::settings_for(exe, false);
        let team = crate::hooks::settings_for(exe, true);
        assert_eq!(plain["hooks"]["PreToolUse"], Value::Null);
        #[cfg(unix)]
        assert_eq!(team["hooks"]["PreToolUse"], json!([{ "hooks": [{ "type": "command", "command": "'/usr/local/bin/pinned' team-guard" }] }]));
        assert_eq!(team["hooks"]["Stop"], plain["hooks"]["Stop"]);
        assert_eq!(team["permissions"], plain["permissions"], "the hook loosens nothing");
    }
}
