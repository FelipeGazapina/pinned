//! `pinned hook pre-tool`: Claude Code's PreToolUse hook in an agent session (a worker, a Captain,
//! the global orchestrator, a workflow run). It runs the never list on the tool call. On a hit, an
//! asking session gets `ask`, so Felipe sees the prompt; a bypass session gets nothing (slice 1 only
//! logs it). Either way the node gets one row for Agent activity. Every error prints nothing, so the
//! normal behavior goes on.

use super::never::{never, Ask, AskCtx, NeverItem};
use crate::agent::{Message, SOCK_ENV, TAB_ENV, TASK_ENV};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The argument after `hook`.
pub const ARG: &str = "pre-tool";

/// What the hook knows besides the payload: its environment and this machine's folders.
#[derive(Debug, Clone, Default)]
pub struct HookEnv {
    /// `CLAUDE_PROJECT_DIR`: the folder the session started in, which a `cd` doesn't move.
    pub project_dir: Option<PathBuf>,
    pub home: PathBuf,
    /// Claude Code's config dir, `~/.claude` unless `CLAUDE_CONFIG_DIR` says otherwise.
    pub config_dir: PathBuf,
    /// Claude Code's per-user temp root, `/tmp/claude-<uid>` resolved.
    pub claude_tmp: Option<PathBuf>,
    pub tab: Option<String>,
    pub task: Option<String>,
}

impl HookEnv {
    fn from_process() -> HookEnv {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let home = crate::machine::home();
        HookEnv {
            project_dir: var("CLAUDE_PROJECT_DIR").map(PathBuf::from),
            config_dir: var("CLAUDE_CONFIG_DIR").map_or_else(|| home.join(".claude"), PathBuf::from),
            claude_tmp: claude_tmp(var("CLAUDE_CODE_TMPDIR")),
            home,
            tab: var(TAB_ENV),
            task: var(TASK_ENV),
        }
    }
}

#[cfg(unix)]
fn claude_tmp(root: Option<String>) -> Option<PathBuf> {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let dir = PathBuf::from(root.unwrap_or_else(|| "/tmp".into())).join(format!("claude-{uid}"));
    Some(std::fs::canonicalize(&dir).unwrap_or(dir))
}

#[cfg(not(unix))]
fn claude_tmp(_root: Option<String>) -> Option<PathBuf> {
    None
}

/// What the hook prints and what it tells the node.
#[derive(Debug)]
pub struct Answer {
    pub stdout: Option<String>,
    pub held: Option<Message>,
}

/// Runs inside the hook process. Never fails loudly.
pub fn run() {
    let mut payload = String::new();
    let _ = std::io::stdin().read_to_string(&mut payload);
    let Some(answer) = answer(&payload, &HookEnv::from_process()) else { return };
    if let Some(out) = &answer.stdout {
        println!("{out}");
    }
    if let (Some(held), Ok(sock)) = (answer.held, std::env::var(SOCK_ENV)) {
        tell(&sock, &held);
    }
}

/// The hook's answer to one PreToolUse payload: None when the call is not on the never list or the
/// payload can't be read.
pub fn answer(payload: &str, env: &HookEnv) -> Option<Answer> {
    let hook: Value = serde_json::from_str(payload).ok()?;
    let tool = hook["tool_name"].as_str()?;
    let input = &hook["tool_input"];
    let ask = Ask::from_tool(tool, input)?;
    let cwd = PathBuf::from(hook["cwd"].as_str()?);
    let start = env.project_dir.clone().unwrap_or_else(|| cwd.clone());
    let here = Repo::of(&cwd);
    let task = Repo::of(&start);
    let worktree = task.as_ref().map_or_else(|| start.clone(), |r| r.top.clone());
    let worktree = std::fs::canonicalize(&worktree).unwrap_or(worktree);
    let branch_repo = here.as_ref().or(task.as_ref());
    let current_branch = branch_repo.and_then(Repo::branch);
    let default_branch = branch_repo.and_then(Repo::default_branch).unwrap_or_else(|| "main".into());
    let session = hook["session_id"].as_str().filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    let scratch = match (&env.claude_tmp, session, slug(&start)) {
        (Some(tmp), Some(session), Some(slug)) => Some(tmp.join(slug).join(session).join("scratchpad")),
        _ => None,
    };
    let main = task.as_ref().and_then(Repo::main_checkout).unwrap_or_else(|| worktree.clone());
    let mut extra = vec![main.join(".claude/research")];
    if let Some(slug) = slug(&main) {
        extra.push(env.config_dir.join("projects").join(slug).join("memory"));
    }
    extra.extend(scratch.clone());
    let ctx = AskCtx {
        worktree: &worktree,
        scratch: scratch.as_deref(),
        home: &env.home,
        cwd: &cwd,
        default_branch: &default_branch,
        current_branch: current_branch.as_deref(),
        extra_writes: &extra,
    };
    let hit = never(&ctx, &ask)?;
    // Only a mode Claude Code names, other than bypass, is an asking session.
    let log_only = hook["permission_mode"].as_str().is_none_or(|mode| mode == "bypassPermissions");
    let stdout = (!log_only).then(|| {
        json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": format!("only Felipe may approve this: {}", hit.item.label()) } }).to_string()
    });
    let held = env.tab.clone().map(|tab| Message::ToolHeld {
        tab,
        tool: tool.to_owned(),
        args: row_args(&ask, input, &hit.why, env.task.as_deref()),
        item: hit.item,
        log_only,
        request_id: hook["tool_use_id"].as_str().map(str::to_owned),
    });
    Some(Answer { stdout, held })
}

/// The row's `args`: the command, path, URL or MCP arguments, why it hit, and the task.
fn row_args(ask: &Ask, input: &Value, why: &str, task: Option<&str>) -> Value {
    let mut args = match ask {
        Ask::Bash { command } => json!({ "command": command }),
        Ask::Edit { path } | Ask::Read { path } => json!({ "path": path.display().to_string() }),
        Ask::WebFetch { url } => json!({ "url": url }),
        Ask::Mcp { .. } => json!({ "input": input }),
    };
    args["why"] = json!(why);
    if let Some(task) = task {
        args["task"] = json!(task);
    }
    args
}

/// The Agent activity detail for a hit.
pub fn detail(item: NeverItem, log_only: bool) -> String {
    if log_only {
        format!("would hold (log only): {}", item.label())
    } else {
        format!("held for Felipe: {}", item.label())
    }
}

fn tell(sock: &str, message: &Message) {
    if let Ok(mut stream) = crate::ipc::UnixStream::connect(sock) {
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        let _ = writeln!(stream, "{}", serde_json::to_string(message).unwrap_or_default());
    }
}

/// Claude Code's folder name for a path: every character but ASCII letters and digits becomes `-`.
/// It hashes a name past 200 characters, which this doesn't follow, so that gives None.
fn slug(path: &Path) -> Option<String> {
    let slug: String = path.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    (slug.len() <= 200).then_some(slug)
}

/// A git checkout, read from its files without running git: the hook runs on every tool call.
struct Repo {
    top: PathBuf,
    gitdir: PathBuf,
    common: PathBuf,
}

impl Repo {
    fn of(start: &Path) -> Option<Repo> {
        for dir in start.ancestors() {
            let dot = dir.join(".git");
            if dot.is_dir() {
                return Some(Repo { top: dir.to_owned(), gitdir: dot.clone(), common: dot });
            }
            if dot.is_file() {
                let text = std::fs::read_to_string(&dot).ok()?;
                let gitdir = dir.join(text.trim().strip_prefix("gitdir:")?.trim());
                let common = std::fs::read_to_string(gitdir.join("commondir")).map_or_else(|_| gitdir.clone(), |c| gitdir.join(c.trim()));
                return Some(Repo { top: dir.to_owned(), gitdir, common });
            }
        }
        None
    }

    fn branch(&self) -> Option<String> {
        let head = std::fs::read_to_string(self.gitdir.join("HEAD")).ok()?;
        head.trim().strip_prefix("ref: refs/heads/").map(str::to_owned)
    }

    fn default_branch(&self) -> Option<String> {
        let head = std::fs::read_to_string(self.common.join("refs/remotes/origin/HEAD")).ok()?;
        head.trim().strip_prefix("ref: refs/remotes/origin/").map(str::to_owned)
    }

    /// The main checkout a linked worktree hangs off.
    fn main_checkout(&self) -> Option<PathBuf> {
        let common = std::fs::canonicalize(&self.common).ok()?;
        (common.file_name()? == ".git").then(|| common.parent().map(Path::to_owned)).flatten()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A main checkout on `fix-0.17.24/foo` with origin/HEAD at main, and a linked worktree of it.
    struct Fx {
        _tmp: tempfile::TempDir,
        main: PathBuf,
        work: PathBuf,
        env: HookEnv,
    }

    fn fx() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(tmp.path()).unwrap();
        let main = base.join("app");
        let work = main.join(".claude/worktrees/w1");
        let git = main.join(".git");
        for d in [git.join("refs/remotes/origin"), git.join("worktrees/w1"), work.join("src"), base.join("home/.claude"), base.join("tmp")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("refs/remotes/origin/HEAD"), "ref: refs/remotes/origin/main\n").unwrap();
        std::fs::write(git.join("worktrees/w1/HEAD"), "ref: refs/heads/fix-0.17.24/foo\n").unwrap();
        std::fs::write(git.join("worktrees/w1/commondir"), "../..\n").unwrap();
        std::fs::write(work.join(".git"), format!("gitdir: {}\n", git.join("worktrees/w1").display())).unwrap();
        let env = HookEnv {
            project_dir: Some(work.clone()),
            home: base.join("home"),
            config_dir: base.join("home/.claude"),
            claude_tmp: Some(base.join("tmp")),
            tab: Some("tab-7".into()),
            task: Some("task-3".into()),
        };
        Fx { _tmp: tmp, main, work, env }
    }

    /// A PreToolUse payload as Claude Code 2.1.322 sends it (captured in a real run).
    fn payload(f: &Fx, mode: &str, tool: &str, input: Value) -> String {
        json!({
            "session_id": "f90c3b0c-c15d-4a75-bd22-a46040d99c38",
            "transcript_path": "/Users/x/.claude/projects/-x/f90c3b0c-c15d-4a75-bd22-a46040d99c38.jsonl",
            "cwd": f.work.display().to_string(),
            "prompt_id": "0e21a80c-7011-4da3-85c6-69b547bcd815",
            "permission_mode": mode,
            "effort": { "level": "medium" },
            "hook_event_name": "PreToolUse",
            "tool_name": tool,
            "tool_input": input,
            "tool_use_id": "toolu_019Wt4UNcXwe4bwzDJvLzCQo"
        })
        .to_string()
    }

    fn bash(f: &Fx, mode: &str, command: &str) -> Option<Answer> {
        answer(&payload(f, mode, "Bash", json!({ "command": command, "description": "x" })), &f.env)
    }

    #[test]
    fn an_asking_session_is_asked_on_a_force_push() {
        let f = fx();
        let got = bash(&f, "default", "git push --force origin fix-0.17.24/foo").unwrap();
        assert_eq!(
            got.stdout.as_deref(),
            Some(r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"only Felipe may approve this: force-push"}}"#)
        );
        assert_eq!(
            serde_json::to_value(got.held.unwrap()).unwrap(),
            json!({
                "kind": "tool-held",
                "tab": "tab-7",
                "tool": "Bash",
                "args": { "command": "git push --force origin fix-0.17.24/foo", "why": "git push --force", "task": "task-3" },
                "item": "force-push",
                "log_only": false,
                "request_id": "toolu_019Wt4UNcXwe4bwzDJvLzCQo",
            })
        );
        assert_eq!(detail(NeverItem::ForcePush, false), "held for Felipe: force-push");
    }

    #[test]
    fn a_bypass_session_prints_nothing_and_records_would_hold() {
        let f = fx();
        let got = bash(&f, "bypassPermissions", "git push --force origin main").unwrap();
        assert_eq!(got.stdout, None);
        let Some(Message::ToolHeld { log_only, item, .. }) = got.held else { panic!("no row: {got:?}") };
        assert!(log_only);
        assert_eq!(detail(item, log_only), "would hold (log only): force-push");
    }

    #[test]
    fn a_harmless_call_and_bad_input_print_nothing() {
        let f = fx();
        assert!(bash(&f, "default", "git status").is_none());
        assert!(bash(&f, "default", "cargo test -p pinned-core").is_none());
        assert!(answer("not json", &f.env).is_none());
        assert!(answer("{}", &f.env).is_none());
        assert!(answer(r#"{"tool_name":"Bash","tool_input":{"command":"git push -f"}}"#, &f.env).is_none(), "no cwd: no answer");
        assert!(answer(&payload(&f, "default", "TodoWrite", json!({})), &f.env).is_none());
    }

    #[test]
    fn the_context_comes_from_the_worktrees_git_files() {
        let f = fx();
        // The worktree is on fix-0.17.24/foo, so a bare push is no push to main.
        assert!(bash(&f, "default", "git push").is_none());
        assert!(bash(&f, "default", "git push origin HEAD:main").is_some());
        // Notes into the main checkout's research dir and the project's memory dir are no write outside.
        let research = f.main.join(".claude/research/0.17.24/x/NOTES.md");
        assert!(answer(&payload(&f, "default", "Write", json!({ "file_path": research, "content": "n" })), &f.env).is_none());
        let memory = f.env.config_dir.join("projects").join(slug(&f.main).unwrap()).join("memory/fact.md");
        assert!(answer(&payload(&f, "default", "Write", json!({ "file_path": memory, "content": "n" })), &f.env).is_none());
        let scratch = f.env.claude_tmp.clone().unwrap().join(slug(&f.work).unwrap()).join("f90c3b0c-c15d-4a75-bd22-a46040d99c38/scratchpad/a.txt");
        assert!(answer(&payload(&f, "default", "Write", json!({ "file_path": scratch, "content": "n" })), &f.env).is_none());
        // The rest of the main checkout is outside the worktree.
        let elsewhere = answer(&payload(&f, "default", "Write", json!({ "file_path": f.main.join("src/lib.rs"), "content": "n" })), &f.env).unwrap();
        assert_eq!(elsewhere.held.map(|m| matches!(m, Message::ToolHeld { item: NeverItem::WriteOutside, .. })), Some(true));
        // A cd elsewhere doesn't move the worktree: CLAUDE_PROJECT_DIR does not follow it.
        let mut moved = json!({ "tool_name": "Bash", "tool_input": { "command": "touch x" }, "cwd": f.main.display().to_string(), "permission_mode": "default" });
        assert!(answer(&moved.to_string(), &f.env).is_some(), "touch in the main checkout writes outside the worktree");
        moved["cwd"] = json!(f.work.join("src").display().to_string());
        assert!(answer(&moved.to_string(), &f.env).is_none());
    }

    #[test]
    fn an_unknown_permission_mode_only_logs() {
        let f = fx();
        let got = answer(&json!({ "tool_name": "Bash", "tool_input": { "command": "git push -f" }, "cwd": f.work.display().to_string() }).to_string(), &f.env).unwrap();
        assert_eq!(got.stdout, None);
    }

    #[test]
    fn a_missing_socket_drops_the_row_and_prints_only_the_answer() {
        let f = fx();
        let got = bash(&f, "default", "git push --force").unwrap();
        // `run` sends the row with `tell`, which gives up silently on a socket that is gone.
        tell("/nonexistent/pinned.sock", got.held.as_ref().unwrap());
        assert!(got.stdout.is_some());
        let no_tab = HookEnv { tab: None, ..f.env.clone() };
        let got = answer(&payload(&f, "default", "Bash", json!({ "command": "git push --force" })), &no_tab).unwrap();
        assert!(got.held.is_none(), "no tab, no row");
    }
}
