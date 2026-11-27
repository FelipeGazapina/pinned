//! `pinned worker-guard`: Claude Code's PreToolUse hook on every Bash call of a worker (#538). A
//! worker tab (`PINNED_ROLE=worker`) loads it from `hooks::WORKER_SETTINGS`, and a headless Claude
//! delegated task from an inline `--settings` that runs it as `worker-guard headless`. Owner
//! threads, Captains, the global Orchestrator and reviewers don't run it. It reads the command
//! against a fixed pattern list and makes no model, socket, hub or network call.
//!
//! Three classes are held for the owner, checked in this order:
//! 1. a delete outside the worktree, as the never list finds it (the session's scratch folder,
//!    `<claude tmp>/<slug>/<session_id>`, counts as inside). A `git clean` or `git rm` is such a
//!    delete when it runs outside, and so is a `git rm` on a line that names a repository to git or
//!    moves it with `env`. A delete strictly below a temp folder (`$TMPDIR`, and `/tmp` on Unix)
//!    runs only on a line that is nothing but `rm`, `rmdir` or `unlink` of literal absolute paths
//!    there: no other command, no expansion, substitution, group or redirect, and no `..`, relative
//!    path or `~` (see `never::temp_line`). Inside that rule a name right under the folder that
//!    starts with `pinned` or `claude` is held, and so is a glob outside the last name, or `*`. A
//!    temp folder that holds the home, the worktree, the session's start folder or its Claude home
//!    does not count. An `mv` into a temp folder is held too, unless its source is in the worktree or
//!    the scratch folder, or is one a temp delete may take;
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

use super::never::{never, path_comps, walk, Ask, AskCtx, NeverItem};
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
    /// The temp folders. A delete may run strictly below those that hold nothing of the session's.
    pub temp: Vec<PathBuf>,
}

impl GuardEnv {
    fn from_process(headless: bool) -> GuardEnv {
        GuardEnv {
            headless,
            project_dir: std::env::var("CLAUDE_PROJECT_DIR").ok().filter(|v| !v.is_empty()).map(PathBuf::from),
            home: crate::machine::home(),
            claude_home: std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).filter(|p| p.is_absolute()),
            claude_tmp: crate::machine::claude_tmp(),
            temp: std::iter::once(std::env::temp_dir()).chain(cfg!(unix).then(|| PathBuf::from("/tmp"))).collect(),
        }
    }
}

/// The `roots` a delete may run strictly below: each an absolute folder that holds none of `keep`,
/// in any case of its letters.
fn temp_roots(roots: &[PathBuf], keep: &[&Path]) -> Vec<PathBuf> {
    let real = |p: &Path| path_comps(&walk(p));
    let keep: Vec<Vec<String>> = keep.iter().map(|p| real(p)).collect();
    roots.iter().filter(|root| root.is_absolute() && !keep.iter().any(|kept| kept.starts_with(&real(root)))).cloned().collect()
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
    let mut keep = vec![env.home.as_path(), &worktree, &start];
    keep.extend(env.claude_home.as_deref());
    let temp = temp_roots(&env.temp, &keep);
    let ctx = AskCtx { worktree: &worktree, scratch: scratch.as_deref(), temp: &temp, home: &env.home, claude_home: env.claude_home.as_deref(), cwd: &cwd, default_branch: "main", current_branch: None };
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
        /// The temp root a delete may run strictly below.
        temp: PathBuf,
        env: GuardEnv,
    }

    /// A worktree holding `.git`, a home, a temp root, a folder outside all three, and session `s1`'s
    /// scratch folder.
    fn fx(headless: bool) -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(tmp.path()).unwrap();
        let (work, home, elsewhere, claude_tmp, temp) = (base.join("work"), base.join("home"), base.join("elsewhere"), base.join("claude-tmp"), base.join("tmp"));
        let scratch = claude_tmp.join(crate::machine::claude_slug(&work)).join("s1");
        for dir in [work.join(".git"), home.join(".ssh"), elsewhere.clone(), scratch.clone(), temp.join("x")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let env = GuardEnv { headless, project_dir: Some(work.clone()), home: home.clone(), claude_home: None, claude_tmp, temp: vec![temp.clone()] };
        Fx { _tmp: tmp, work, home, elsewhere, scratch, temp, env }
    }

    /// `s` with each `{name}` a path of the fixture.
    #[cfg(unix)]
    fn sub(f: &Fx, s: &str) -> String {
        let show = |p: &Path| p.display().to_string();
        s.replace("{work}", &show(&f.work)).replace("{home}", &show(&f.home)).replace("{elsewhere}", &show(&f.elsewhere)).replace("{scratch}", &show(&f.scratch)).replace("{tmp}", &show(&f.temp))
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

    /// Each command asks the owner about a delete.
    #[cfg(unix)]
    fn asks(f: &Fx, env: &GuardEnv, commands: &[&str]) {
        let asked = ("ask".to_owned(), "pinned holds this command for the owner (delete outside the worktree).".to_owned());
        for command in commands {
            let command = sub(f, command);
            assert_eq!(held(decide(&bash(f, &command, Some("default")), env)), Some(asked.clone()), "{command}");
        }
    }

    #[cfg(unix)]
    fn runs(f: &Fx, env: &GuardEnv, commands: &[&str]) {
        for command in commands {
            let command = sub(f, command);
            assert_eq!(decide(&bash(f, &command, Some("default")), env), None, "{command}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_git_rm_is_a_delete_in_the_repository_it_runs_in() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "cd {elsewhere} && git ls-files -z | xargs -0 git rm -q -f",
            "cd {elsewhere} && env git rm -r -f .",
            "git -C {elsewhere} rm -f x",
            "cd {elsewhere} && git rm -q x",
            "cd {elsewhere} && nice git rm -r .",
            "cd \"$D\" && git rm -r .",
            "env git --work-tree={elsewhere} --git-dir={elsewhere}/.git rm -r -f .",
            "GIT_WORK_TREE={elsewhere} git rm -r .",
            "sudo -g git rm -rf {home}",
            "env -C {elsewhere} git rm -r -f sub",
            "env --chdir={elsewhere} git rm -r -f sub",
        ]);
        runs(&f, &f.env, &[
            "git ls-files -z | xargs -0 git rm -q -f",
            "git rm -q old.rs",
            "cd sub && env git rm -r old",
            "git -C {work} rm -f x",
            "git rm -q \"$f\"",
        ]);
    }

    /// Each command asks the owner about a delete outside the worktree, with `cwd` as the call's cwd.
    #[cfg(unix)]
    fn asks_at(f: &Fx, env: &GuardEnv, cwd: &Path, commands: &[&str]) {
        let asked = ("ask".to_owned(), "pinned holds this command for the owner (delete outside the worktree).".to_owned());
        for command in commands {
            let command = sub(f, command);
            let payload = json!({ "session_id": "s1", "tool_name": "Bash", "tool_input": { "command": command }, "cwd": cwd, "permission_mode": "default" }).to_string();
            assert_eq!(held(decide(&payload, env)), Some(asked.clone()), "{command}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_plain_temp_delete_runs() {
        let f = fx(false);
        runs(&f, &f.env, &[
            "rm -f {tmp}/x-unused",
            "rm -rf {tmp}/pg.Kmep {tmp}/x/*.log {tmp}/x-pgdir.txt",
            "rm -rf {tmp}/x/a && rm -f {tmp}/x/b.log",
            "rm -rf -- {tmp}/x/a; rm -f {tmp}/x/b",
            "rm -rf \"{tmp}/x/a\"",
            "rm -rf '{tmp}/x/a'",
            "rmdir {tmp}/x/empty",
            "unlink {tmp}/x/f",
        ]);
    }

    /// A `mv` that brings outside data into a temp folder is held: a later line's temp delete would
    /// otherwise remove what the owner had outside it. A `ln` is not: a delete walks a link to its target.
    #[test]
    #[cfg(unix)]
    fn a_move_that_brings_outside_data_into_temp_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "mv {elsewhere} {tmp}/x/o",
            "mv a {elsewhere}/b {tmp}/x",
            "mv -t {tmp}/x {elsewhere}/f",
            "mv {elsewhere} \"$TMPDIR/o\"",
            "mv \"$d\" {tmp}/x/",
            "echo {elsewhere} | xargs -I {} mv {} {tmp}/x/",
            "mv {tmp}/pinned-1-a {tmp}/x/p",
            "mv {tmp}/* {tmp}/x/",
        ]);
        runs(&f, &f.env, &[
            "ln -s {home} {tmp}/x/a",
            "ln {elsewhere}/f {tmp}/x/h",
            "ln -s ../../elsewhere {tmp}/x/a",
            "cd {tmp}/x && ln -s {home}",
            "mv sub {tmp}/x/m",
            "ln -s {work}/sub {tmp}/x/a",
            "ln -s b {tmp}/x/a",
            "mv {tmp}/x/a.log {tmp}/x/b.log",
            "mv {tmp}/x/*.log {tmp}/x/old",
            "mv {scratch}/a {tmp}/x/",
            "mv -t {tmp}/x sub/a.log",
            "mv a b",
            "mv {elsewhere}/f {work}/",
            "for f in *.js; do mv \"$f\" \"${f%.js}.ts\"; done",
            "cp -R {elsewhere} {tmp}/x/copy",
        ]);
    }

    /// Hole 1: a `cd` in the same line runs in another process, so the delete does not follow it.
    #[test]
    #[cfg(unix)]
    fn hole_1_a_cd_joined_by_background_pipe_or_or_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &["cd {tmp}/x & rm -rf ../src", "cd {tmp}/x | rm -rf ../src", "cd {tmp}/nope || rm -rf ../src"]);
    }

    /// Hole 2: the shell's extglob runs under `shopt`, and the lexer ends a word at `(`.
    #[test]
    #[cfg(unix)]
    fn hole_2_an_extglob_pattern_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &["shopt -s extglob; rm -rf {tmp}/!(pinned)", "cd {tmp} && rm -rf !(pinned)"]);
    }

    /// Hole 3: a program named by a variable or a substitution is not `ln`, whatever it expands to.
    #[test]
    #[cfg(unix)]
    fn hole_3_a_program_named_by_a_variable_or_a_substitution_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "L=ln; $L -s ~ {tmp}/x/l && rm -rf {tmp}/x/l/*",
            "\"${L:-ln}\" -s ~ {tmp}/x/l && rm -rf {tmp}/x/l/*",
            "$(echo ln) -s ~ {tmp}/x/l && rm -rf {tmp}/x/l/*",
        ]);
    }

    /// Hole 4: a command before the delete can write a tree that the delete then reaches.
    #[test]
    #[cfg(unix)]
    fn hole_4_a_build_or_unpack_before_the_delete_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "./w.sh && rm -rf {tmp}/x/l/*",
            "make && rm -rf {tmp}/x/l/*",
            "git clone ./r {tmp}/x/c && rm -rf {tmp}/x/c/l/*",
            "unzip a.zip -d {tmp}/x && rm -rf {tmp}/x/l/*",
            "tar -xf a.tar -C {tmp}/x && rm -rf {tmp}/x/a/*",
        ]);
    }

    /// Hole 5: `find -exec mv` moves a file out of temp. Held on main too.
    #[test]
    #[cfg(unix)]
    fn hole_5_find_exec_mv_out_of_a_temp_folder_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &["find {tmp}/x -exec mv {} ~/.ssh/ \\;"]);
    }

    #[test]
    #[cfg(unix)]
    fn a_temp_delete_beside_another_command_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "ln -s {home} {tmp}/x/a && rm -rf {tmp}/x/a/*",
            "ln -s {work}/sub {tmp}/x/a && rm -rf {tmp}/x/a/*",
            "rm -rf {tmp}/x/a/*; ln -s {work}/sub {tmp}/x/a",
            "mv sub {tmp}/x/m && rm -rf {tmp}/x/m",
            "cp -R sub {tmp}/x/c && rm -rf {tmp}/x/c",
            "install -d {tmp}/x/i && rm -rf {tmp}/x/i",
            "rsync -a sub/ {tmp}/x/r/ && rm -rf {tmp}/x/r",
            "sh make.sh && rm -f {tmp}/x/b",
            "bash make.sh; rm -f {tmp}/x/b",
            "zsh make.sh && find {tmp}/x/b -delete",
            "bash -c 'rm -rf {tmp}/x/b'",
            "python make.py && rm -f {tmp}/x/b",
            "node make.js && rm -f {tmp}/x/b",
            "eval make && rm -f {tmp}/x/b",
            "source env.sh && rm -f {tmp}/x/b",
            "sudo rm -rf {tmp}/x/a",
            "env rm -rf {tmp}/x/a",
        ]);
    }

    #[test]
    #[cfg(unix)]
    fn a_temp_delete_in_a_group_redirect_or_second_line_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "rm -rf {tmp}/x/a > {tmp}/x/log",
            "(rm -rf {tmp}/x/a)",
            "rm -rf {tmp}/x/a; cargo test",
            "cargo test && rm -rf {tmp}/x/a",
            "rm -rf {tmp}/x/{a,b}",
            "rm -rf {tmp}/x/a\nrm -rf {home}/x",
            "rm -rf {tmp}/x/$F",
            "rm -rf {tmp}/x/../..",
            "rm -rf ~/tmp/x",
        ]);
    }

    #[test]
    #[cfg(unix)]
    fn a_temp_delete_by_shred_or_find_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &["shred -u {tmp}/x/a", "find {tmp}/x -delete", "find {tmp}/x -type f -exec shred -u {} +"]);
        runs(&f, &f.env, &["shred -u secret.bin"]);
    }

    #[test]
    #[cfg(unix)]
    fn a_relative_delete_in_a_temp_folder_is_held() {
        let f = fx(false);
        asks_at(&f, &f.env, &f.temp, &["rm -rf a", "rm -rf -- -x"]);
        asks(&f, &f.env, &["cd {tmp}/x && rm -rf a"]);
    }

    #[test]
    #[cfg(unix)]
    fn a_rmdir_with_parents_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &["rmdir -p {tmp}/x/a/b"]);
    }

    #[test]
    #[cfg(unix)]
    fn a_temp_path_that_resolves_outside_the_temp_root_is_held() {
        let f = fx(false);
        std::os::unix::fs::symlink(&f.elsewhere, f.temp.join("x/out")).unwrap();
        std::os::unix::fs::symlink(&f.home, f.temp.join("x/l")).unwrap();
        asks(&f, &f.env, &["rm -rf {tmp}/x/out/*", "rm -rf {tmp}/x/out/a", "rm -rf {tmp}/x/l/*", "rm -f {tmp}/x/l"]);
    }

    #[test]
    #[cfg(unix)]
    fn a_temp_name_starting_pinned_or_claude_is_held_at_any_root() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "rm -rf {tmp}/pinned-1",
            "rm -rf {tmp}/pinned-1/x",
            "rm -rf {tmp}/Claude-501/x",
            "rm -f {tmp}/Pinned-1-a/claude-settings-worker.json",
        ]);
        // A temp root inside another one: the name right under the outer root counts too.
        let nested = GuardEnv { temp: vec![f.temp.join("claude-501"), f.temp.clone()], ..f.env.clone() };
        asks(&f, &nested, &["rm -rf {tmp}/claude-501/s2"]);
    }

    #[test]
    #[cfg(unix)]
    fn a_temp_glob_sits_in_the_last_name_and_is_not_bare() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "rm -rf {tmp}/x*",
            "rm -rf {tmp}/*",
            "rm -rf {tmp}/x/*",
            "rm -rf {tmp}/x/.*",
            "rm -rf {tmp}/x/.hidden*",
            "rm -rf {tmp}/*/a",
            "rm -rf {tmp}/x/*/pg",
        ]);
    }

    #[test]
    #[cfg(unix)]
    fn a_temp_root_or_its_contents_as_a_whole_is_held() {
        let f = fx(false);
        asks(&f, &f.env, &["rm -rf {tmp}", "rm -rf {tmp}/x/a/../.."]);
    }

    #[test]
    #[cfg(unix)]
    fn a_temp_root_that_holds_the_worktree_or_the_claude_home_is_dropped() {
        let f = fx(false);
        std::fs::create_dir_all(f.temp.join("proj/wt/.git")).unwrap();
        // `TMPDIR` is the folder that holds the worktree.
        let worktree = GuardEnv { project_dir: Some(f.temp.join("proj/wt")), temp: vec![f.temp.join("proj")], ..f.env.clone() };
        asks(&f, &worktree, &["rm -rf {tmp}/proj/other"]);
        let claude = GuardEnv { claude_home: Some(f.temp.join("accounts/claude")), ..f.env.clone() };
        asks(&f, &claude, &["rm -f {tmp}/x/a.log"]);
    }

    /// A delete that only a session's own scratch folder holds runs as before, relative or not.
    #[test]
    #[cfg(unix)]
    fn a_delete_in_the_session_scratch_folder_runs() {
        let f = fx(false);
        let claude_tmp = f.temp.join("claude-501");
        let own = claude_tmp.join(crate::machine::claude_slug(&f.work)).join("s1");
        let env = GuardEnv { claude_tmp, ..f.env.clone() };
        runs(&f, &env, &[&format!("cd {} && rm *.o", own.display())]);
    }

    #[test]
    fn a_temp_root_that_holds_home_is_dropped() {
        let f = fx(false);
        let above_home = f.home.parent().unwrap().to_owned();
        let roots = [f.temp.clone(), above_home, f.home.clone(), PathBuf::from("tmp")];
        assert_eq!(temp_roots(&roots, &[&f.home]), [f.temp]);
    }

    /// Each command is held for `label`, the reason a hit names, with `cwd` as the call's cwd.
    #[cfg(unix)]
    fn asks_for(f: &Fx, env: &GuardEnv, label: &str, commands: &[&str]) {
        let asked = ("ask".to_owned(), format!("pinned holds this command for the owner ({label})."));
        for command in commands {
            let command = sub(f, command);
            assert_eq!(held(decide(&bash(f, &command, Some("default")), env)), Some(asked.clone()), "{command}");
        }
    }

    /// `!(` opens a negated subshell, so what it holds is commands, whatever word comes before it.
    #[test]
    #[cfg(unix)]
    fn a_negated_subshell_is_read_as_commands() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "!(rm -rf ~/Documents)",
            "true && !(rm -rf ~/Documents)",
            "if !(rm -rf ~/Documents); then :; fi",
            "(!(rm -rf ~/Documents))",
            "x=$(!(rm -rf ~/Documents))",
            "if !(grep -q '(' notes.txt); then :; fi\nrm -rf ~/Documents",
            "!(echo \")\"); rm -rf ~/Documents",
            "bash -c '!(rm -rf ~/Documents)'",
            "f?() { rm -rf ~/Documents; }",
        ]);
        asks_for(&f, &f.env, "force-push", &["!(git push --force origin main)"]);
        asks(&f, &f.env, &["rm -rf {tmp}/x/!(keep)"]);
    }

    /// A function whose name ends in an extglob group still runs its body, so its `rm` is a command.
    #[test]
    #[cfg(unix)]
    fn a_function_named_with_an_extglob_runs_its_body() {
        let f = fx(false);
        asks(&f, &f.env, &["x/@() { rm -rf ~/Documents; }; x/@", "f ./@() rm -rf ~/Documents; f", "function x/@() { rm -rf ~/.ssh; }"]);
        asks_for(&f, &f.env, "force-push", &["f x/!() { git push --force origin main; }; f"]);
    }

    /// `\r` ends no word in the shell, so `a\r#` is one word and the `#` after it is no comment.
    #[test]
    #[cfg(unix)]
    fn a_carriage_return_does_not_end_a_word() {
        let f = fx(false);
        asks(&f, &f.env, &["rm -f {tmp}/x/a\r#; rm -rf ~/Documents", "true\r#; rm -rf ~/Documents", "rm -f {tmp}/x/a\r#; git push --force origin main"]);
        asks_for(&f, &f.env, "force-push", &["true\r#; git push --force origin main"]);
    }

    /// macOS `rm` and `rmdir` stop reading options at the first operand, so a later `-…` word is a file.
    #[test]
    #[cfg(unix)]
    fn a_dash_word_after_the_first_path_is_a_file() {
        let f = fx(false);
        asks(&f, &f.env, &["rm -rf {tmp}/x/a -Documents", "rm -rf {tmp}/x/a -d/../../../Users/x", "rmdir {tmp}/x/a -victim"]);
        runs(&f, &f.env, &["rm -rf -- {tmp}/x/a {tmp}/x/b", "rm -f -r {tmp}/x/a"]);
    }

    /// Under a wrapper, `git` hides the `rm` after it only when nothing else stands between them, so
    /// any other word there (an option, a value, `NAME=V`) makes the `rm` the program.
    #[test]
    #[cfg(unix)]
    fn git_as_an_options_value_does_not_hide_rm() {
        let f = fx(false);
        asks(&f, &f.env, &[
            "echo x | xargs -I git rm -rf ~/Documents",
            "echo x | xargs -E git rm -rf ~/Documents",
            "echo x | xargs -J git rm -rf ~/Documents",
            "exec -a git rm -rf ~/Documents",
            "parallel -I git rm -rf ~/Documents ::: a",
            "sudo -p git rm -rf ~/Documents",
            "arch -arm64 -d git rm -rf ~/Documents",
            "env -iC git rm -rf ~/Documents",
            "env -i -C git rm -rf ~/Documents",
            "env --unset git rm -rf ~/Documents",
            "echo x | xargs --arg-file git rm -rf ~/Documents",
            "echo x | xargs --process-slot-var git rm -rf ~/Documents",
            "xargs arch -arm64 -d git rm -rf ~/Documents",
        ]);
        runs(&f, &f.env, &["git ls-files -z | xargs -0 git rm -q -f", "git ls-files -z | xargs -0 -r git rm -q -f", "env git rm -q x", "nice git rm -q x"]);
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
