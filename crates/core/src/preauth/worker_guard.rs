//! `pinned worker-guard`: Claude Code's PreToolUse hook on every Bash call of a worker (#538). A
//! worker tab (`PINNED_ROLE=worker`) loads it from `hooks::WORKER_SETTINGS`, and a headless Claude
//! delegated task from an inline `--settings` that runs it as `worker-guard headless`. Owner
//! threads, Captains, the global Orchestrator and reviewers don't run it. It reads the command
//! against a fixed pattern list and makes no model, socket, hub or network call.
//!
//! Three classes are held for the owner, checked in this order:
//! 1. a delete outside the worktree, as the never list finds it (the session's scratch folder,
//!    `<claude tmp>/<slug>/<session_id>`, counts as inside);
//! 2. a force-push, as the never list finds it, a branch delete by push included;
//! 3. dropping a database: `dropdb`, or a SQL client named in a command whose text drops a
//!    database, a schema or a table, or truncates one.
//!
//! The rest of the never list (push to main, deploy, release, secrets ...) runs as before.
//!
//! On a hit, a worker that can show Claude Code's permission prompt gets it (`ask`), and Needs you
//! lists it as an approval. A headless worker (`claude -p`), or a session whose mode is `dontAsk`,
//! missing or unknown, has the command refused (`deny`), with a reason that tells it to report the
//! step as needing the owner.
//!
//! Manual check, Claude Code 2.1.331: a `--settings` with a Bash PreToolUse hook that always
//! printed `ask`, asked to run `echo hi`. Launch 1, default mode with the allow rule
//! `Bash(echo:*)`: the prompt showed. Launch 2, `--dangerously-skip-permissions` (the payload's
//! mode is `bypassPermissions`): the prompt showed. Launch 3, `--permission-mode auto`: the prompt
//! showed. So `bypassPermissions` and `auto` ask too.

use super::never::{never, Ask, AskCtx, NeverItem};
use super::team_hook::repo_top;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

pub const SUBCOMMAND: &str = "worker-guard";
/// The argument a headless worker's hook runs with: no prompt can show there.
pub const HEADLESS: &str = "headless";

const DROP_DATABASE: &str = "drop a database";
/// The modes whose session shows the prompt a hook's `ask` asks for, by the manual check above.
const PROMPTING: [&str; 5] = ["default", "acceptEdits", "plan", "bypassPermissions", "auto"];
/// The longest command a deny reason repeats whole.
const SHOWN_CHARS: usize = 300;
const SQL_CLIENTS: [&str; 14] =
    ["psql", "pgcli", "mysql", "mariadb", "mycli", "sqlite3", "sqlite", "litecli", "duckdb", "sqlcmd", "clickhouse", "clickhouse-client", "cockroach", "usql"];

/// What the hook knows besides the payload.
#[derive(Debug, Clone, Default)]
pub struct GuardEnv {
    /// The session runs as `claude -p`, where no prompt can show.
    pub headless: bool,
    /// `CLAUDE_PROJECT_DIR`: where the session started, which a `cd` does not move.
    pub project_dir: Option<PathBuf>,
    pub home: PathBuf,
    /// `CLAUDE_CONFIG_DIR`, when absolute: the session's own Claude home.
    pub claude_home: Option<PathBuf>,
    /// Where Claude Code keeps each session's scratchpad.
    pub claude_tmp: PathBuf,
}

impl GuardEnv {
    fn from_process(headless: bool) -> GuardEnv {
        GuardEnv {
            headless,
            project_dir: std::env::var("CLAUDE_PROJECT_DIR").ok().filter(|v| !v.is_empty()).map(PathBuf::from),
            home: crate::machine::home(),
            claude_home: std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).filter(|p| p.is_absolute()),
            claude_tmp: crate::machine::claude_tmp(),
        }
    }
}

/// Runs inside the hook process. Prints an ask or a deny, or nothing so the call runs as before.
pub fn run(first: Option<&str>) {
    let env = GuardEnv::from_process(first == Some(HEADLESS));
    if let Some(output) = decide(&crate::hooks::hook_payload(), &env) {
        println!("{output}");
    }
}

/// The hook's stdout for one PreToolUse payload: `None` for any call but a Bash command in one of
/// the three classes, else an ask or a deny.
pub fn decide(payload: &str, env: &GuardEnv) -> Option<String> {
    let hook: Value = serde_json::from_str(payload).ok()?;
    if hook["tool_name"] != "Bash" {
        return None;
    }
    let command = hook["tool_input"]["command"].as_str()?;
    let cwd = PathBuf::from(hook["cwd"].as_str()?);
    let start = env.project_dir.clone().unwrap_or_else(|| cwd.clone());
    let worktree = repo_top(&start).unwrap_or_else(|| start.clone());
    let worktree = std::fs::canonicalize(&worktree).unwrap_or(worktree);
    // A session id is one plain name; anything else would point the scratch folder elsewhere.
    let session = hook["session_id"].as_str().filter(|id| Path::new(id).components().eq([Component::Normal(id.as_ref())]));
    let scratch = session.map(|id| env.claude_tmp.join(crate::machine::claude_slug(&start)).join(id));
    let ctx = AskCtx { worktree: &worktree, scratch: scratch.as_deref(), home: &env.home, claude_home: env.claude_home.as_deref(), cwd: &cwd, default_branch: "main", current_branch: None };
    let label = match never(&ctx, &Ask::Bash { command }) {
        Some(hit) if matches!(hit.item, NeverItem::DeleteOutside | NeverItem::ForcePush) => hit.item.label(),
        _ if drops_database(command) => DROP_DATABASE,
        _ => return None,
    };
    let prompts = !env.headless && hook["permission_mode"].as_str().is_some_and(|mode| PROMPTING.contains(&mode));
    let (decision, reason) = if prompts {
        ("ask", format!("pinned holds this command for the owner ({label})."))
    } else {
        let reason = format!(
            "pinned held this command for the owner ({label}): {}\nOnly the owner may approve it. Do not try to do this step another way. Report this step as needing the owner, then go on with the rest of your task.",
            shown(command)
        );
        ("deny", reason)
    };
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": decision, "permissionDecisionReason": reason } }).to_string())
}

/// `command` as written, cut to its first `SHOWN_CHARS` characters with `…` when it is longer.
fn shown(command: &str) -> String {
    match command.char_indices().nth(SHOWN_CHARS) {
        Some((cut, _)) => format!("{}…", &command[..cut]),
        None => command.to_owned(),
    }
}

/// Whether `command` drops a database: `dropdb`, or a SQL client beside a SQL drop or truncate
/// anywhere in its text, so a pipe, a heredoc, `docker exec` or a nested shell counts too.
fn drops_database(command: &str) -> bool {
    let words: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || "'\"`;|&()<>".contains(c))
        .map(|piece| piece.rsplit('/').next().unwrap_or(piece))
        .collect();
    words.contains(&"dropdb") || (words.iter().any(|word| SQL_CLIENTS.contains(word)) && drops_sql(command))
}

/// Whether `text`, in any case, holds `DROP DATABASE`, `DROP SCHEMA` or `DROP TABLE`, or a
/// `TRUNCATE` followed by something other than a flag (the shell's `truncate -s 0 file` is none).
fn drops_sql(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let text = lower.as_str();
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    // What follows each whole word `name` that whitespace follows, that whitespace cut.
    let after = |name: &'static str| {
        text.match_indices(name).filter_map(move |(at, _)| {
            let rest = &text[at + name.len()..];
            let next = rest.trim_start();
            (!text[..at].ends_with(word) && next.len() < rest.len()).then_some(next)
        })
    };
    after("drop").any(|next| ["database", "schema", "table"].iter().any(|kind| next.strip_prefix(kind).is_some_and(|tail| !tail.starts_with(word))))
        || after("truncate").any(|next| next.chars().next().is_some_and(|c| c != '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "Only the owner may approve it. Do not try to do this step another way. Report this step as needing the owner, then go on with the rest of your task.";

    #[cfg_attr(not(unix), allow(dead_code))]
    struct Fx {
        _tmp: tempfile::TempDir,
        work: PathBuf,
        home: PathBuf,
        elsewhere: PathBuf,
        /// The session's scratch folder, under the Claude temp root.
        scratch: PathBuf,
        env: GuardEnv,
    }

    /// A worktree holding `.git`, a home, a folder outside both, and session `s1`'s scratch folder.
    fn fx(headless: bool) -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(tmp.path()).unwrap();
        let (work, home, elsewhere, claude_tmp) = (base.join("work"), base.join("home"), base.join("elsewhere"), base.join("claude-tmp"));
        let scratch = claude_tmp.join(crate::machine::claude_slug(&work)).join("s1");
        for dir in [work.join(".git"), home.join(".ssh"), elsewhere.clone(), scratch.clone()] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let env = GuardEnv { headless, project_dir: Some(work.clone()), home: home.clone(), claude_home: None, claude_tmp };
        Fx { _tmp: tmp, work, home, elsewhere, scratch, env }
    }

    /// `s` with each `{name}` a path of the fixture.
    #[cfg(unix)]
    fn sub(f: &Fx, s: &str) -> String {
        let show = |p: &Path| p.display().to_string();
        s.replace("{work}", &show(&f.work)).replace("{home}", &show(&f.home)).replace("{elsewhere}", &show(&f.elsewhere)).replace("{scratch}", &show(&f.scratch))
    }

    fn bash(f: &Fx, command: &str, mode: Option<&str>) -> String {
        let mut payload = json!({ "session_id": "s1", "tool_name": "Bash", "tool_input": { "command": command }, "cwd": f.work });
        if let Some(mode) = mode {
            payload["permission_mode"] = json!(mode);
        }
        payload.to_string()
    }

    /// The decision and reason of a hook's output.
    fn held(out: Option<String>) -> Option<(String, String)> {
        let out: Value = serde_json::from_str(&out?).unwrap();
        let specific = &out["hookSpecificOutput"];
        assert_eq!(specific["hookEventName"], "PreToolUse");
        Some((specific["permissionDecision"].as_str().unwrap().to_owned(), specific["permissionDecisionReason"].as_str().unwrap().to_owned()))
    }

    #[test]
    #[cfg(unix)]
    fn a_headless_worker_is_refused_each_class_with_a_reason_to_report() {
        let f = fx(true);
        for (command, label) in [
            ("rm -rf ~", "delete outside the worktree"),
            ("rm -rf {elsewhere}", "delete outside the worktree"),
            ("git push --force origin feat", "force-push"),
            ("git push --force-with-lease", "force-push"),
            ("git push origin :feat", "force-push"),
            ("dropdb app", "drop a database"),
            ("psql \"$DATABASE_URL\" -c 'drop table users'", "drop a database"),
            ("mysql -e \"DROP DATABASE app\"", "drop a database"),
            ("echo 'DROP SCHEMA s CASCADE' | psql", "drop a database"),
            ("psql <<'SQL'\nTRUNCATE users;\nSQL", "drop a database"),
            ("docker exec db psql -c \"DROP TABLE x\"", "drop a database"),
            ("bash -c \"psql -c 'DROP TABLE x'\"", "drop a database"),
        ] {
            let command = sub(&f, command);
            let reason = format!("pinned held this command for the owner ({label}): {command}\n{REPORT}");
            assert_eq!(held(decide(&bash(&f, &command, Some("default")), &f.env)), Some(("deny".into(), reason)), "{command}");
        }
    }

    #[test]
    fn an_interactive_worker_gets_the_prompt_in_every_mode_that_shows_it() {
        let f = fx(false);
        let push = "git push --force origin feat";
        for mode in ["default", "acceptEdits", "plan", "bypassPermissions", "auto"] {
            let asked = ("ask".to_owned(), "pinned holds this command for the owner (force-push).".to_owned());
            assert_eq!(held(decide(&bash(&f, push, Some(mode)), &f.env)), Some(asked), "{mode}");
        }
        for mode in [Some("dontAsk"), None, Some("something-new")] {
            let refused = format!("pinned held this command for the owner (force-push): {push}\n{REPORT}");
            assert_eq!(held(decide(&bash(&f, push, mode), &f.env)), Some(("deny".into(), refused)), "{mode:?}");
        }
        let (decision, _) = held(decide(&bash(&f, "dropdb app", Some("default")), &f.env)).unwrap();
        assert_eq!(decision, "ask");
    }

    #[test]
    #[cfg(unix)]
    fn every_other_call_runs_as_before() {
        let f = fx(true);
        for command in [
            "cargo test",
            "rm -rf target",
            "rm -rf {scratch}/x",
            "git push origin feat",
            "git push origin main",
            "vercel deploy --prod",
            "ssh host",
            "npm install left-pad",
            "psql -c 'select 1'",
            "truncate -s 0 log.txt",
            "grep -rn \"DROP TABLE\" migrations",
        ] {
            let command = sub(&f, command);
            assert_eq!(decide(&bash(&f, &command, Some("default")), &f.env), None, "{command}");
        }
        let cwd = f.work.display().to_string();
        for payload in [
            json!({ "tool_name": "Write", "tool_input": { "file_path": f.elsewhere.join("x.txt"), "content": "x" }, "cwd": cwd, "permission_mode": "default" }).to_string(),
            json!({ "tool_name": "Read", "tool_input": { "file_path": f.home.join(".ssh/id_ed25519") }, "cwd": cwd, "permission_mode": "default" }).to_string(),
            json!({ "tool_name": "Bash", "tool_input": { "command": "rm -rf ~" }, "permission_mode": "default" }).to_string(),
            "not json".to_owned(),
        ] {
            assert_eq!(decide(&payload, &f.env), None, "{payload}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn only_the_sessions_own_scratch_folder_counts_as_inside() {
        let f = fx(true);
        let other = bash(&f, &sub(&f, "rm -rf {scratch}/x"), Some("default")).replace("\"s1\"", "\"s2\"");
        assert!(decide(&other, &f.env).is_some(), "another session's scratch folder is outside");
        let climbing = bash(&f, &sub(&f, "rm -rf {home}/x"), Some("default")).replace("\"s1\"", "\"../../home\"");
        assert!(decide(&climbing, &f.env).is_some(), "a session id is no path");
    }

    #[test]
    fn a_long_command_is_cut_in_the_reason() {
        let f = fx(true);
        let long = format!("dropdb app {}", "é".repeat(400));
        let (decision, reason) = held(decide(&bash(&f, &long, Some("default")), &f.env)).unwrap();
        assert_eq!(decision, "deny");
        let cut: String = long.chars().take(300).collect();
        assert_eq!(reason, format!("pinned held this command for the owner (drop a database): {cut}…\n{REPORT}"));
        let exact = format!("dropdb {}", "a".repeat(293));
        let (_, reason) = held(decide(&bash(&f, &exact, Some("default")), &f.env)).unwrap();
        assert!(reason.contains(&format!("{exact}\n")), "300 characters stay whole");
    }
}
