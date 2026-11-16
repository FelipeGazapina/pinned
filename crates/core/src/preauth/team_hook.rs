//! `pinned team-guard`: Claude Code's PreToolUse hook in a team's box. A never-list hit (a deploy,
//! a release, a push to main ...) is the kind of step only people may approve, and a team's bot has
//! no user at its terminal. So the hook asks the team (`TeamOp::AskPeople`, `BeforeIrreversible`)
//! and denies the tool call until the answer comes back as a message. It fails closed: when the
//! hub can't be asked, the step is denied.
//!
//! The question shows the exact step (secrets hidden), and carries the hash of the exact tool call
//! (`step_hash`). A "Go ahead" lets that one tool call run once, in the same run, within 10
//! minutes: when the bot makes it again, the hub answers `granted` and the hook lets it through.
//! Any other step, or the same one again, is asked again.
//!
//! Outside a team box (`PINNED_TEAM_BOX` not `1`) the hook does nothing, and no settings file
//! registers it: the never list is still only a pure function there.

use super::never::{never, Ask, AskCtx};
use crate::agent::Message;
use crate::team_call;
use crate::teams::Situation;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const SUBCOMMAND: &str = "team-guard";

/// What a session reads when the hub could not be asked.
pub const COULD_NOT_ASK: &str = "Could not ask the team; do not do this step.";
/// The choices every step's question offers.
pub const GO_AHEAD: &str = "Go ahead";
pub const DONT: &str = "Don't";
/// What a session reads for a step longer than `STEP_CHARS`, which nobody is asked about.
pub const TOO_LONG: &str = "This step is too long to show to the team; split it into shorter steps.";

/// What a session reads for a step that sets a secret-named variable to a command: hiding the
/// value would hide the command from the person asked, so nobody is asked and the step is denied.
pub const HIDES_COMMAND: &str = "this step hides a command in a secret";

/// The longest step, secrets hidden, a person is asked to approve whole.
const STEP_CHARS: usize = 4000;

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
    let tool = hook["tool_name"].as_str()?;
    if hides_command(&step_text(tool, &tool_ask, input)) {
        return Some(deny(HIDES_COMMAND));
    }
    let shown = step(tool, &tool_ask, input);
    if shown.chars().count() > STEP_CHARS {
        return Some(deny(TOO_LONG));
    }
    let question = format!("{}: {shown}", hit.item.label());
    let message = Message::AskPeople { situation: Situation::BeforeIrreversible, question, choices: vec![GO_AHEAD.into(), DONT.into()], step: Some(step_hash(tool, input)) };
    match ask(message).and_then(|reply| team_call::parse_asked(&reply)) {
        Ok(asked) if asked.granted => None,
        Ok(asked) => Some(deny(&format!(
            "Asked the team before this step (call {}). End your turn and wait for the answer; only if it says go ahead, make this exact tool call again, unchanged.",
            asked.id
        ))),
        Err(_) => Some(deny(COULD_NOT_ASK)),
    }
}

fn deny(reason: &str) -> String {
    json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } }).to_string()
}

/// The hex SHA-256 of `tool` and its input with every object's keys sorted, before anything is
/// hidden: the same tool call hashes the same, a byte changed anywhere does not.
pub fn step_hash(tool: &str, input: &Value) -> String {
    Sha256::digest(format!("{tool}\n{}", canonical(input))).iter().map(|b| format!("{b:02x}")).collect()
}

/// `value` as JSON with every object's keys sorted.
fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&map[k]))).collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    sorted(value).to_string()
}

/// The exact step as it will run, with tokens and secret-named assignments hidden: a command as
/// the bot wrote it, any other tool call as its name and input.
fn step(tool: &str, ask: &Ask, input: &Value) -> String {
    scrub(&step_text(tool, ask, input))
}

/// The step as it will run, nothing hidden.
fn step_text(tool: &str, ask: &Ask, input: &Value) -> String {
    match ask {
        Ask::Bash { command } => (*command).to_owned(),
        Ask::Edit { .. } | Ask::Read { .. } | Ask::WebFetch { .. } | Ask::Mcp { .. } => format!("{tool} {}", canonical(input)),
    }
}

/// `text` whole, with keys and tokens hidden, and the value of any `NAME=value` whose name says
/// secret. Every space and line break stays where it was.
fn scrub(text: &str) -> String {
    crate::surface::mask_secrets(text)
        .split_inclusive(char::is_whitespace)
        .map(|piece| {
            let word = piece.trim_end();
            match word.split_once('=') {
                Some((name, value)) if !value.is_empty() && secret_name(name) => format!("{name}=[hidden]{}", &piece[word.len()..]),
                _ => piece.to_owned(),
            }
        })
        .collect()
}

/// Whether `text` sets a secret-named `NAME=` to anything but a plain literal (`$(...)`, a
/// backtick, `;`, a quote ...): `scrub` would hide that part, so a person approving the step would
/// not see the command in it.
fn hides_command(text: &str) -> bool {
    let literal = |c: char| c.is_ascii_alphanumeric() || "_./+:@%-".contains(c);
    text.split(char::is_whitespace).any(|word| match word.split_once('=') {
        Some((name, value)) => !value.is_empty() && secret_name(name) && !value.chars().all(literal),
        None => false,
    })
}

/// Whether the variable name `name` says secret: its uppercase, after any leading `-`, contains a
/// secret word anywhere. Broad on purpose: `DBPASSWORD`, `MYTOKEN` and `AUTHOR` all do, since
/// hiding too much only costs a person a look at the step.
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
        assert_eq!(
            denied(&out.unwrap()),
            "Asked the team before this step (call k7). End your turn and wait for the answer; only if it says go ahead, make this exact tool call again, unchanged."
        );
        let Some(Message::AskPeople { situation, question, choices, step }) = message else { panic!("no ask") };
        assert_eq!(situation, Situation::BeforeIrreversible);
        assert_eq!(choices, ["Go ahead", "Don't"]);
        assert_eq!(question, "deploy: vercel deploy --prod --token vca_abc");
        assert_eq!(step, Some(step_hash("Bash", &json!({ "command": "vercel deploy --prod --token vca_abc" }))));
    }

    #[test]
    fn a_granted_reply_lets_the_step_run_with_no_output() {
        let deploy = payload("Bash", json!({ "command": "vercel deploy --prod" }));
        let (out, message) = decision(&deploy, true, Ok(r#"{"granted":true,"call":"k7"}"#));
        assert_eq!(out, None);
        assert!(matches!(message, Some(Message::AskPeople { step: Some(_), .. })), "the hub was asked, with the step");
        for not_granted in [r#"{"granted":false,"call":"k7"}"#, r#"{"call":"k7","asked":["the owner"]}"#] {
            let (out, _) = decision(&deploy, true, Ok(not_granted));
            assert!(denied(&out.unwrap()).starts_with("Asked the team before this step (call k7)."), "{not_granted}");
        }
    }

    #[test]
    fn the_step_hash_is_the_same_for_the_same_call_and_differs_by_one_byte() {
        let input = json!({ "command": "vercel deploy --prod", "description": "Deploy", "timeout": 60000 });
        let reordered: Value = serde_json::from_str(r#"{"timeout":60000,"description":"Deploy","command":"vercel deploy --prod"}"#).unwrap();
        let hash = step_hash("Bash", &input);
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{hash}");
        assert_eq!(step_hash("Bash", &input), hash);
        assert_eq!(step_hash("Bash", &reordered), hash, "the keys' order does not count");
        assert_ne!(step_hash("Bash", &json!({ "command": "vercel deploy --prod ", "description": "Deploy", "timeout": 60000 })), hash);
        assert_ne!(step_hash("Bash", &json!({ "command": "vercel deploy --prod", "description": "Deploy", "timeout": 60001 })), hash);
        assert_ne!(step_hash("mcp__x__Bash", &input), hash);
        // The hash is of the input as it runs, not of what the person reads with secrets hidden.
        let with_token = |token: &str| step_hash("Bash", &json!({ "command": format!("vercel deploy --prod --token vca_{token}") }));
        assert_ne!(with_token("abc"), with_token("abd"));
    }

    #[test]
    fn the_whole_step_is_asked_and_a_step_too_long_to_show_is_denied_without_asking() {
        let long = format!("vercel deploy --prod --build-env NOTE={}", "a".repeat(3900));
        let (out, message) = decision(&payload("Bash", json!({ "command": long })), true, Ok(r#"{"id":"k1"}"#));
        assert!(out.is_some());
        let Some(Message::AskPeople { question, .. }) = message else { panic!("no ask") };
        assert_eq!(question, format!("deploy: {long}"), "no cut at 300 characters");
        let multi = "vercel deploy --prod \\\n  --yes";
        let (_, message) = decision(&payload("Bash", json!({ "command": multi })), true, Ok(r#"{"id":"k1"}"#));
        assert!(matches!(message, Some(Message::AskPeople { question, .. }) if question == format!("deploy: {multi}")), "line breaks stay");

        let too_long = format!("vercel deploy --prod --build-env NOTE={}", "a".repeat(4000));
        let (out, message) = decision(&payload("Bash", json!({ "command": too_long })), true, Ok(r#"{"id":"k1"}"#));
        assert_eq!(denied(&out.unwrap()), "This step is too long to show to the team; split it into shorter steps.");
        assert!(message.is_none(), "nobody is asked");
        // 4000 characters is the most, counted as characters, not bytes.
        let at_most = format!("vercel deploy --prod --x {}", "é".repeat(4000 - 25));
        let (_, message) = decision(&payload("Bash", json!({ "command": at_most })), true, Ok(r#"{"id":"k1"}"#));
        assert!(message.is_some());
    }

    #[test]
    fn a_tool_call_other_than_a_command_is_shown_as_its_name_and_input() {
        let read = payload("Read", json!({ "file_path": "/home/box/.ssh/id_ed25519" }));
        let (out, message) = decision(&read, true, Ok(r#"{"id":"k1"}"#));
        assert!(out.is_some());
        let Some(Message::AskPeople { question, .. }) = message else { panic!("no ask") };
        assert_eq!(question, r#"secrets: Read {"file_path":"/home/box/.ssh/id_ed25519"}"#);
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
        assert_eq!(scrub("a  TOKEN=x\n\tb=c "), "a  TOKEN=[hidden]\n\tb=c ");
        assert_eq!(scrub(&"a ".repeat(400)), "a ".repeat(400));
    }

    #[test]
    fn a_secret_set_to_a_command_is_denied_and_nobody_is_asked() {
        for command in [
            "SECRET_KEY=$(git push -f origin main) vercel deploy --prod",
            "API_TOKEN=`git push -f origin main` vercel deploy --prod",
            "AUTH=x;git push -f origin main; vercel deploy --prod",
            "GH_TOKEN=\"$(cat ~/.ssh/id_ed25519)\" gh release create v1",
            "vercel deploy --prod --token=a|sh",
        ] {
            let (out, message) = decision(&payload("Bash", json!({ "command": command })), true, Ok(r#"{"id":"k1"}"#));
            assert_eq!(denied(&out.unwrap()), "this step hides a command in a secret", "{command}");
            assert!(message.is_none(), "nobody is asked: {command}");
        }
        // A plain literal is still hidden and asked about.
        let (_, message) = decision(&payload("Bash", json!({ "command": "SECRET_KEY=abc.def/1+2:x@y%z-_ vercel deploy --prod" })), true, Ok(r#"{"id":"k1"}"#));
        let Some(Message::AskPeople { question, .. }) = message else { panic!("no ask") };
        assert_eq!(question, "deploy: SECRET_KEY=[hidden] vercel deploy --prod");
        // A one-word name is hidden from the members asked, and from the notice built from it.
        let (_, message) = decision(&payload("Bash", json!({ "command": "DBPASSWORD=hunter2 vercel deploy --prod" })), true, Ok(r#"{"id":"k1"}"#));
        let Some(Message::AskPeople { question, .. }) = message else { panic!("no ask") };
        assert_eq!(question, "deploy: DBPASSWORD=[hidden] vercel deploy --prod");
        assert_eq!(scrub("STRIPEKEY=abc MYTOKEN=xyz"), "STRIPEKEY=[hidden] MYTOKEN=[hidden]");
        // A name that contains a secret word anywhere is a secret: hiding more is safer.
        let (out, message) = decision(&payload("Bash", json!({ "command": "AUTHOR=$(whoami) MONKEY=x vercel deploy --prod" })), true, Ok(r#"{"id":"k1"}"#));
        assert_eq!(denied(&out.unwrap()), "this step hides a command in a secret");
        assert!(message.is_none());
        assert_eq!(scrub("AUTHOR=x MONKEY=y KEY=z"), "AUTHOR=[hidden] MONKEY=[hidden] KEY=[hidden]");
        assert!(hides_command("AUTHOR=$(x) MONKEY=`y` NOTE=a;b"));
        assert!(!hides_command("NOTE=$(x) USER=`y`"));
        assert!(hides_command("--password='$(x)'"));
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
