//! What each session changed in a project, kept as the diff of every turn.
//!
//! A turn's diff is the difference between two snapshots of the working tree, one taken when the
//! prompt was submitted and one when the turn ended. A snapshot is a git tree written through a
//! throwaway index, so the user's own index and branches are never touched.

use crate::process;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// A turn's patch is cut here, so one huge generated file can't bloat the history.
const PATCH_LIMIT: usize = 200_000;
const PROMPT_CHARS: usize = 400;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// Every field but `session` has a default, so a line written by another version still loads.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Turn {
    /// Claude Code's session id.
    pub session: String,
    #[serde(default)]
    pub title: Option<String>,
    /// What the user asked, cut short.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Unix seconds the turn ended.
    #[serde(default)]
    pub ended: u64,
    #[serde(default)]
    pub files: Vec<FileChange>,
    /// Unified diff of the whole turn.
    #[serde(default)]
    pub patch: String,
    /// The patch hit `PATCH_LIMIT` and lost its tail.
    #[serde(default)]
    pub truncated: bool,
    /// The snapshot the turn ended on, a git tree, to read its files as they were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    /// The snapshot taken as the turn began, which rewinding to its prompt restores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

/// The project a folder belongs to: a `claude --worktree` session's checkout reports to the repo
/// that holds it.
pub fn project_of(cwd: &Path) -> PathBuf {
    let mut parent = cwd;
    while let Some(up) = parent.parent() {
        if up.ends_with(".claude/worktrees") {
            return up.parent().and_then(Path::parent).unwrap_or(cwd).to_owned();
        }
        parent = up;
    }
    cwd.to_owned()
}

/// Where `project`'s turns live under `dir`: one line per turn, appended as they finish.
pub fn file_for(dir: &Path, project: &Path) -> PathBuf {
    // FNV-1a, since `DefaultHasher` may change between Rust versions and orphan old files.
    let hash = project.as_os_str().as_encoded_bytes().iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
    let name = project.file_name().map_or_else(|| "root".into(), |n| n.to_string_lossy().into_owned());
    dir.join(format!("{name}-{hash:016x}.jsonl"))
}

pub fn append(file: &Path, turn: &Turn) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = serde_json::to_string(turn).unwrap();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{line}");
    }
}

/// Every turn saved for the project, newest first. Lines that don't parse are skipped.
pub fn load(file: &Path) -> Vec<Turn> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let mut turns: Vec<Turn> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    turns.reverse();
    turns
}

fn git(cwd: &Path, args: &[&str], index: Option<&Path>) -> Option<Vec<u8>> {
    let mut cmd = process::command("git");
    cmd.args(args).current_dir(cwd).stdin(Stdio::null()).stderr(Stdio::null());
    if let Some(index) = index {
        cmd.env("GIT_INDEX_FILE", index);
    }
    let out = cmd.output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// A tree id holding everything in the working tree that git doesn't ignore, or `None` outside a
/// repository. Run it before the agent can touch files: a hook that waits for it is enough.
pub fn snapshot(cwd: &Path) -> Option<String> {
    let index = std::env::temp_dir().join(format!("pinned-index-{}-{:x}", std::process::id(), nanos()));
    let tree = (|| {
        git(cwd, &["rev-parse", "--is-inside-work-tree"], None)?;
        // A repository with no commit yet has no HEAD to start the index from.
        if git(cwd, &["rev-parse", "--verify", "-q", "HEAD"], None).is_some() {
            git(cwd, &["read-tree", "HEAD"], Some(&index))?;
        }
        git(cwd, &["add", "-A"], Some(&index))?;
        let tree = git(cwd, &["write-tree"], Some(&index))?;
        Some(String::from_utf8_lossy(&tree).trim().to_owned())
    })();
    let _ = std::fs::remove_file(index);
    tree
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

/// What changed between two snapshots, or `None` when nothing did.
pub fn diff(cwd: &Path, before: &str, after: &str) -> Option<(Vec<FileChange>, String, bool)> {
    if before == after {
        return None;
    }
    let stats = git(cwd, &["diff", "--numstat", "--no-renames", "-z", before, after], None)?;
    let files = parse_numstat(&String::from_utf8_lossy(&stats));
    if files.is_empty() {
        return None;
    }
    let mut patch = git(cwd, &["diff", "--no-color", "--find-renames", before, after], None)?;
    let truncated = patch.len() > PATCH_LIMIT;
    if truncated {
        let mut cut = PATCH_LIMIT;
        while cut > 0 && (patch[cut] & 0xC0) == 0x80 {
            cut -= 1;
        }
        patch.truncate(cut);
    }
    Some((files, String::from_utf8_lossy(&patch).into_owned(), truncated))
}

/// Puts the working tree back as it was in `snapshot`: files made since are deleted and the rest get
/// that content back. The index and branches are left alone, as `snapshot` leaves them.
pub fn restore(cwd: &Path, snapshot: &str) -> Result<(), String> {
    let top = git(cwd, &["rev-parse", "--show-toplevel"], None).ok_or("this folder is not in a git repository")?;
    let top = PathBuf::from(String::from_utf8_lossy(&top).trim());
    let now = self::snapshot(&top).ok_or("could not read the working tree")?;
    git(&top, &["cat-file", "-e", &format!("{snapshot}^{{tree}}")], None).ok_or("that checkpoint is gone from the repository")?;
    let names = |filter: &str| -> Result<Vec<String>, String> {
        let out = git(&top, &["diff", "--name-only", "-z", "--no-renames", &format!("--diff-filter={filter}"), snapshot, &now], None).ok_or("could not compare with the checkpoint")?;
        Ok(String::from_utf8_lossy(&out).split('\0').filter(|p| !p.is_empty()).map(str::to_owned).collect())
    };
    for added in names("A")? {
        std::fs::remove_file(top.join(&added)).map_err(|e| format!("could not remove {added}: {e}"))?;
    }
    let changed = names("DMT")?;
    if changed.is_empty() {
        return Ok(());
    }
    let index = std::env::temp_dir().join(format!("pinned-restore-{}-{:x}", std::process::id(), nanos()));
    let restored = (|| {
        git(&top, &["read-tree", snapshot], Some(&index)).ok_or("could not read the checkpoint")?;
        let mut cmd = process::command("git");
        cmd.args(["checkout-index", "-f", "-z", "--stdin"]).current_dir(&top).env("GIT_INDEX_FILE", &index).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        child.stdin.take().unwrap().write_all(changed.join("\0").as_bytes()).map_err(|e| e.to_string())?;
        child.wait().map_err(|e| e.to_string())?.success().then_some(()).ok_or_else(|| "could not write the checkpoint's files".to_owned())
    })();
    let _ = std::fs::remove_file(index);
    restored
}

/// Parses `git diff --numstat -z` with `--no-renames`: `added\tremoved\tpath\0` per file.
/// Binary files report `-` for both counts and count as zero.
fn parse_numstat(out: &str) -> Vec<FileChange> {
    out.split('\0')
        .filter_map(|record| {
            let mut parts = record.trim_start_matches('\n').splitn(3, '\t');
            let (added, removed, path) = (parts.next()?, parts.next()?, parts.next()?);
            Some(FileChange { path: path.to_owned(), added: added.parse().unwrap_or(0), removed: removed.parse().unwrap_or(0) })
        })
        .collect()
}

/// The prompt as stored: its first `PROMPT_CHARS` characters.
pub fn clip_prompt(prompt: &str) -> String {
    let prompt = prompt.trim();
    match prompt.char_indices().nth(PROMPT_CHARS) {
        Some((cut, _)) => format!("{}…", &prompt[..cut]),
        None => prompt.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pinned-history-test-{}-{name}", std::process::id()))
    }

    fn run(cwd: &Path, args: &[&str]) {
        let ok = process::command("git").args(args).current_dir(cwd).output().unwrap().status.success();
        assert!(ok, "git {args:?} failed");
    }

    fn repo(name: &str) -> PathBuf {
        let dir = temp(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run(&dir, &["init", "-q"]);
        run(&dir, &["config", "user.email", "t@t"]);
        run(&dir, &["config", "user.name", "t"]);
        run(&dir, &["config", "core.autocrlf", "false"]);
        dir
    }

    #[test]
    fn a_turn_diff_holds_only_what_the_turn_changed() {
        let dir = repo("turn");
        std::fs::write(dir.join("kept.txt"), "one\n").unwrap();
        std::fs::write(dir.join("edited.txt"), "a\nb\n").unwrap();
        run(&dir, &["add", "."]);
        run(&dir, &["commit", "-qm", "init"]);
        // Dirty before the turn starts: the turn must not claim this.
        std::fs::write(dir.join("kept.txt"), "one\ntwo\n").unwrap();
        let before = snapshot(&dir).unwrap();

        std::fs::write(dir.join("edited.txt"), "a\nc\nd\n").unwrap();
        std::fs::write(dir.join("new.txt"), "hello\n").unwrap();
        let after = snapshot(&dir).unwrap();

        let (files, patch, truncated) = diff(&dir, &before, &after).unwrap();
        assert_eq!(
            files,
            vec![
                FileChange { path: "edited.txt".into(), added: 2, removed: 1 },
                FileChange { path: "new.txt".into(), added: 1, removed: 0 },
            ]
        );
        assert!(patch.contains("+c\n") && patch.contains("+hello\n") && !patch.contains("two"));
        assert!(!truncated);
        assert_eq!(diff(&dir, &after, &snapshot(&dir).unwrap()), None);
        std::fs::rename(dir.join("edited.txt"), dir.join("renamed.txt")).unwrap();
        let (_, moved, _) = diff(&dir, &after, &snapshot(&dir).unwrap()).unwrap();
        assert!(moved.contains("rename from edited.txt\nrename to renamed.txt"), "{moved}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn restoring_a_snapshot_undoes_edits_additions_and_deletions_but_not_the_index() {
        let dir = repo("restore");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/kept.txt"), "one\n").unwrap();
        std::fs::write(dir.join("gone.txt"), "bye\n").unwrap();
        run(&dir, &["add", "."]);
        run(&dir, &["commit", "-qm", "init"]);
        std::fs::write(dir.join("dirty.txt"), "mine\n").unwrap();
        let before = snapshot(&dir).unwrap();

        std::fs::write(dir.join("src/kept.txt"), "two\n").unwrap();
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        std::fs::write(dir.join("src/new.txt"), "new\n").unwrap();
        std::fs::write(dir.join("dirty.txt"), "changed\n").unwrap();
        restore(&dir.join("src"), &before).unwrap();

        let read = |p: &str| std::fs::read_to_string(dir.join(p)).ok();
        assert_eq!([read("src/kept.txt"), read("gone.txt"), read("src/new.txt"), read("dirty.txt")], [Some("one\n".into()), Some("bye\n".into()), None, Some("mine\n".into())]);
        let staged = process::command("git").args(["diff", "--cached", "--name-only"]).current_dir(&dir).output().unwrap().stdout;
        assert!(staged.is_empty());
        assert!(restore(&dir, "0123456789abcdef0123456789abcdef01234567").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn snapshots_work_before_the_first_commit_and_leave_the_index_alone() {
        let dir = repo("unborn");
        std::fs::write(dir.join("a.txt"), "x\n").unwrap();
        let before = snapshot(&dir).unwrap();
        std::fs::write(dir.join("b.txt"), "y\n").unwrap();
        let (files, ..) = diff(&dir, &before, &snapshot(&dir).unwrap()).unwrap();
        assert_eq!(files, vec![FileChange { path: "b.txt".into(), added: 1, removed: 0 }]);
        let staged = process::command("git").args(["diff", "--cached", "--name-only"]).current_dir(&dir).output().unwrap().stdout;
        assert!(staged.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn folders_outside_a_repository_have_no_snapshot() {
        let dir = temp("plain");
        std::fs::create_dir_all(&dir).unwrap();
        // /tmp can itself sit inside a repository on odd setups; only assert when it doesn't.
        if git(&dir, &["rev-parse", "--is-inside-work-tree"], None).is_none() {
            assert_eq!(snapshot(&dir), None);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn worktree_sessions_belong_to_the_repo_that_holds_them() {
        assert_eq!(project_of(Path::new("/repo/.claude/worktrees/eng-7")), PathBuf::from("/repo"));
        assert_eq!(project_of(Path::new("/repo/.claude/worktrees/eng-7/src")), PathBuf::from("/repo"));
        assert_eq!(project_of(Path::new("/repo/src")), PathBuf::from("/repo/src"));
    }

    #[test]
    fn turns_append_and_load_newest_first() {
        let file = temp("turns.jsonl");
        let _ = std::fs::remove_file(&file);
        let turn = |session: &str, ended| Turn { session: session.into(), title: None, prompt: Some("p".into()), ended, files: vec![], patch: String::new(), truncated: false, tree: None, before: None };
        append(&file, &turn("s1", 1));
        append(&file, &turn("s2", 2));
        std::fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(b"{broken\n{\"session\":\"s3\"}\n").unwrap();
        let sessions: Vec<String> = load(&file).into_iter().map(|t| t.session).collect();
        assert_eq!(sessions, vec!["s3", "s2", "s1"]);
        assert_eq!(load(Path::new("/nonexistent/h.jsonl")), vec![]);
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn long_prompts_are_clipped() {
        assert_eq!(clip_prompt("  fix it \n"), "fix it");
        assert_eq!(clip_prompt(&"é".repeat(500)).chars().count(), PROMPT_CHARS + 1);
    }

    #[test]
    fn binary_files_count_as_zero_lines() {
        assert_eq!(
            parse_numstat("-\t-\timg.png\03\t1\tsrc/a b.rs\0"),
            vec![
                FileChange { path: "img.png".into(), added: 0, removed: 0 },
                FileChange { path: "src/a b.rs".into(), added: 3, removed: 1 },
            ]
        );
    }
}
