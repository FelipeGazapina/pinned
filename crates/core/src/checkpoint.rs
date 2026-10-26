//! A workflow session's work as another machine needs it to resume: the conversation, and the
//! code with whatever was not pushed yet, uncommitted files included.

use crate::wire::Bytes;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// Set on the checkpoint commit so capturing works on a machine with no git identity configured.
const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "pinned"),
    ("GIT_AUTHOR_EMAIL", "pinned@localhost"),
    ("GIT_COMMITTER_NAME", "pinned"),
    ("GIT_COMMITTER_EMAIL", "pinned@localhost"),
];
/// Holds the fetched checkpoint commit, whose tree the restored worktree is reset to.
const RESTORED: &str = "refs/pinned/restore";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// Claude Code's id for the conversation.
    pub session: String,
    /// The session's transcript, as Claude Code wrote it.
    pub transcript: Bytes,
    /// The worktree the session worked in, by its name under `.claude/worktrees`, if it used one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Code {
    /// A git bundle holding `commit` and every commit under it that no remote has.
    pub bundle: Bytes,
    /// A commit of the whole working tree on top of HEAD.
    pub commit: String,
    /// The branch HEAD was on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// The transcript of `session`, wherever Claude Code keeps it under `home`.
pub fn transcript(home: &Path, session: &str) -> Option<PathBuf> {
    let file = format!("{session}.jsonl");
    std::fs::read_dir(home.join(".claude/projects"))
        .ok()?
        .filter_map(|entry| {
            let path = entry.ok()?.path().join(&file);
            let meta = path.metadata().ok().filter(std::fs::Metadata::is_file)?;
            Some((meta.modified().ok()?, path))
        })
        .max()
        .map(|(_, path)| path)
}

/// The folder the session last worked in, as its transcript records it.
pub fn session_cwd(transcript: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(transcript).ok()?;
    String::from_utf8_lossy(&bytes).lines().rev().find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()?.get("cwd")?.as_str().map(PathBuf::from))
}

/// The work in `cwd`, or None outside a repository, before its first commit, or when the
/// bundle would pass `limit` bytes.
pub fn capture_code(cwd: &Path, limit: u64) -> Option<Code> {
    let head = git(cwd, &["rev-parse", "--verify", "-q", "HEAD"]).ok()?;
    let tree = crate::history::snapshot(cwd)?;
    // A signing setup could prompt or fail on a machine nobody is watching.
    let commit = git_with(cwd, &["commit-tree", "--no-gpg-sign", &tree, "-p", &head, "-m", "pinned checkpoint"], &IDENTITY).ok()?;
    let id = format!("{}-{:x}", std::process::id(), nanos());
    let name = format!("refs/pinned/checkpoint-{id}");
    let file = std::env::temp_dir().join(format!("pinned-checkpoint-{id}.bundle"));
    let bundle = (|| {
        git(cwd, &["update-ref", &name, &commit]).ok()?;
        git(cwd, &["bundle", "create", "-q", file.to_str()?, &name, "--not", "--remotes"]).ok()?;
        if std::fs::metadata(&file).ok()?.len() > limit {
            return None;
        }
        std::fs::read(&file).ok()
    })();
    let _ = git(cwd, &["update-ref", "-d", &name]);
    let _ = std::fs::remove_file(&file);
    let branch = git(cwd, &["symbolic-ref", "--short", "-q", "HEAD"]).ok();
    Some(Code { bundle: Bytes(bundle?), commit, branch })
}

/// Puts the checkpoint's work in the worktree `.claude/worktrees/<name>` of `checkout`, replacing
/// one an earlier attempt left, and its transcript where Claude Code looks for that folder's
/// sessions. Answers the folder to resume in.
pub fn restore(home: &Path, checkout: &Path, name: &str, checkpoint: &Checkpoint) -> Result<PathBuf, String> {
    // Both come from another machine and become paths; one that climbs out could delete the checkout.
    let (name, session) = (plain(name)?, plain(&checkpoint.session)?);
    let target = checkout.join(".claude/worktrees").join(name);
    let path = target.to_str().ok_or("the worktree's path is not UTF-8")?;
    let base = match &checkpoint.code {
        Some(code) => {
            fetch(checkout, code)?;
            format!("{}^", code.commit)
        }
        None => "HEAD".to_owned(),
    };
    let _ = git(checkout, &["worktree", "remove", "--force", path]);
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("could not clear {path}: {e}"))?;
    }
    // After the folder is gone, so a worktree git could not remove is dropped from its list too.
    git(checkout, &["worktree", "prune"])?;
    git(checkout, &["worktree", "add", "-q", "--detach", path, &base])?;
    if let Some(code) = &checkpoint.code {
        if let Some(branch) = &code.branch {
            adopt(&target, branch);
        }
        crate::history::restore(&target, &format!("{}^{{tree}}", code.commit))?;
    }
    let dir = crate::sync::claude_dir(home, &target);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    std::fs::write(dir.join(format!("{session}.jsonl")), &checkpoint.transcript.0).map_err(|e| format!("could not write the transcript: {e}"))?;
    Ok(target)
}

/// Fetches the checkpoint commit into `checkout` as `RESTORED`.
fn fetch(checkout: &Path, code: &Code) -> Result<(), String> {
    let file = std::env::temp_dir().join(format!("pinned-restore-{}-{:x}.bundle", std::process::id(), nanos()));
    std::fs::write(&file, &code.bundle.0).map_err(|e| format!("could not write the bundle: {e}"))?;
    let fetched = (|| {
        let path = file.to_str().ok_or("the bundle's path is not UTF-8")?;
        let heads = git(checkout, &["bundle", "list-heads", path])?;
        let head = heads.lines().find_map(|l| l.strip_prefix(code.commit.as_str())?.strip_prefix(' ')).ok_or("the bundle does not hold the checkpoint's commit")?;
        let refspec = format!("+{head}:{RESTORED}");
        // The bundle leaves out what the capturing machine's remotes had, which this clone may lag.
        git(checkout, &["fetch", "-q", path, &refspec]).or_else(|_| {
            git(checkout, &["fetch", "-q", "--all"])?;
            git(checkout, &["fetch", "-q", path, &refspec])
        })
    })();
    let _ = std::fs::remove_file(&file);
    fetched.map(drop)
}

/// Puts the worktree on `branch`, unless another worktree has it checked out or moving it to HEAD
/// would drop commits this machine made on it. The worktree stays detached then.
fn adopt(worktree: &Path, branch: &str) {
    let local = format!("refs/heads/{branch}");
    if git(worktree, &["rev-parse", "--verify", "-q", &local]).is_ok() && git(worktree, &["merge-base", "--is-ancestor", &local, "HEAD"]).is_err() {
        return;
    }
    let _ = git(worktree, &["checkout", "-q", "-B", branch]);
}

fn plain(name: &str) -> Result<&str, String> {
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(_)), None) => Ok(name),
        _ => Err(format!("{name:?} is not a plain name")),
    }
}

fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    git_with(cwd, args, &[])
}

fn git_with(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let out = Command::new("git").args(args).envs(env.iter().copied()).current_dir(cwd).stdin(Stdio::null()).output().map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-checkpoint-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(cwd: &Path, args: &[&str]) -> String {
        let out = Command::new("git").args(args).current_dir(cwd).output().unwrap();
        assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    }

    fn repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        run(dir, &["init", "-q", "-b", "main"]);
        run(dir, &["config", "user.email", "t@t"]);
        run(dir, &["config", "user.name", "t"]);
        run(dir, &["config", "commit.gpgsign", "false"]);
    }

    fn commit(dir: &Path, file: &str, text: &str, message: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "-qm", message]);
    }

    fn checkpoint(code: Option<Code>) -> Checkpoint {
        Checkpoint { session: "s1".into(), transcript: Bytes(b"{\"cwd\":\"/x\"}\n".to_vec()), worktree: Some("w".into()), code }
    }

    /// Everything a resumed session would see in `dir`.
    fn state(dir: &Path) -> [String; 6] {
        let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap_or_else(|_| "<missing>".into());
        [
            run(dir, &["log", "--format=%s"]),
            run(dir, &["symbolic-ref", "--short", "-q", "HEAD"]),
            run(dir, &["status", "--porcelain"]),
            read("tracked.txt"),
            read("untracked.txt"),
            read("other.txt"),
        ]
    }

    #[test]
    fn a_checkpoint_carries_unpushed_commits_edits_and_new_files_to_another_clone() {
        let root = temp("carry");
        let remote = root.join("remote.git");
        run(&root, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        let a = root.join("a");
        repo(&a);
        commit(&a, "tracked.txt", "v1\n", "init");
        run(&a, &["remote", "add", "origin", remote.to_str().unwrap()]);
        run(&a, &["push", "-q", "-u", "origin", "main"]);
        let b = root.join("b");
        run(&root, &["clone", "-q", remote.to_str().unwrap(), b.to_str().unwrap()]);
        // Pushed after b cloned, so b has to fetch it before the bundle applies.
        commit(&a, "other.txt", "second\n", "second");
        run(&a, &["push", "-q", "origin", "main"]);
        run(&a, &["checkout", "-q", "-b", "feature"]);
        commit(&a, "tracked.txt", "v2\n", "feature work");
        std::fs::write(a.join("tracked.txt"), "v3 uncommitted\n").unwrap();
        std::fs::write(a.join("untracked.txt"), "new\n").unwrap();
        std::fs::remove_file(a.join("other.txt")).unwrap();
        let before = run(&a, &["status", "--porcelain"]);

        let code = capture_code(&a, 1 << 20).unwrap();
        assert_eq!(code.branch.as_deref(), Some("feature"));
        assert_eq!(run(&a, &["rev-parse", &format!("{}^", code.commit)]), run(&a, &["rev-parse", "HEAD"]));
        assert_eq!(run(&a, &["status", "--porcelain"]), before);
        assert_eq!(run(&a, &["for-each-ref", "refs/pinned"]), "");

        let home = root.join("home");
        let cp = checkpoint(Some(code));
        let target = restore(&home, &b, "w", &cp).unwrap();
        assert_eq!(target, b.join(".claude/worktrees/w"));
        let expected = [
            "feature work\nsecond\ninit".to_owned(),
            "feature".into(),
            " D other.txt\n M tracked.txt\n?? untracked.txt".into(),
            "v3 uncommitted\n".into(),
            "new\n".into(),
            "<missing>".into(),
        ];
        assert_eq!(state(&target), expected);
        let transcript = crate::sync::claude_dir(&home, &target).join("s1.jsonl");
        assert_eq!(std::fs::read_to_string(&transcript).unwrap(), "{\"cwd\":\"/x\"}\n");

        std::fs::write(target.join("junk.txt"), "left by the first attempt\n").unwrap();
        std::fs::write(target.join("tracked.txt"), "drifted\n").unwrap();
        assert_eq!(restore(&home, &b, "w", &cp).unwrap(), target);
        assert_eq!(state(&target), expected);
        assert!(!target.join("junk.txt").exists());
        assert_eq!(run(&b, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn without_code_the_worktree_starts_from_the_checkouts_head() {
        let root = temp("nocode");
        let b = root.join("b");
        repo(&b);
        commit(&b, "tracked.txt", "v1\n", "init");
        let home = root.join("home");
        let target = restore(&home, &b, "w", &checkpoint(None)).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("tracked.txt")).unwrap(), "v1\n");
        assert_eq!(run(&target, &["rev-parse", "HEAD"]), run(&b, &["rev-parse", "HEAD"]));
        assert!(Command::new("git").args(["symbolic-ref", "-q", "HEAD"]).current_dir(&target).status().unwrap().code() == Some(1));
        assert_eq!(std::fs::read(crate::sync::claude_dir(&home, &target).join("s1.jsonl")).unwrap(), b"{\"cwd\":\"/x\"}\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn names_that_leave_the_worktrees_folder_are_refused() {
        let root = temp("escape");
        let b = root.join("b");
        repo(&b);
        commit(&b, "tracked.txt", "v1\n", "init");
        let home = root.join("home");
        assert_eq!(restore(&home, &b, "../..", &checkpoint(None)), Err("\"../..\" is not a plain name".into()));
        let mut cp = checkpoint(None);
        cp.session = "../s1".into();
        assert_eq!(restore(&home, &b, "w", &cp), Err("\"../s1\" is not a plain name".into()));
        assert_eq!(std::fs::read_to_string(b.join("tracked.txt")).unwrap(), "v1\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nothing_is_captured_without_a_commit_or_past_the_limit() {
        let root = temp("none");
        // The temp folder can itself sit inside a repository on odd setups; only assert when it doesn't.
        if git(&root, &["rev-parse", "--is-inside-work-tree"]).is_err() {
            assert_eq!(capture_code(&root, u64::MAX), None);
        }
        let a = root.join("a");
        repo(&a);
        std::fs::write(a.join("tracked.txt"), "v1\n").unwrap();
        assert_eq!(capture_code(&a, u64::MAX), None);
        commit(&a, "tracked.txt", "v1\n", "init");
        assert_eq!(capture_code(&a, 10), None);
        assert_eq!(run(&a, &["for-each-ref", "refs/pinned"]), "");
        run(&a, &["checkout", "-q", "--detach"]);
        let code = capture_code(&a, u64::MAX).unwrap();
        assert_eq!(code.branch, None);
        assert_eq!(&code.bundle.0[..16], b"# v2 git bundle\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_session_cwd_is_the_last_one_recorded() {
        let root = temp("cwd");
        let file = root.join("s.jsonl");
        std::fs::write(&file, "{\"cwd\":\"/first\"}\n{\"type\":\"summary\"}\n{broken\n{\"cwd\":\"/repo/.claude/worktrees/w\",\"x\":1}\n{\"type\":\"assistant\"}\n").unwrap();
        assert_eq!(session_cwd(&file), Some(PathBuf::from("/repo/.claude/worktrees/w")));
        std::fs::write(&file, "{\"type\":\"summary\"}\n").unwrap();
        assert_eq!(session_cwd(&file), None);
        assert_eq!(session_cwd(&root.join("missing.jsonl")), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_newest_transcript_of_a_session_wins() {
        let home = temp("find");
        let projects = home.join(".claude/projects");
        for (slug, age) in [("-old", 100), ("-new", 10)] {
            std::fs::create_dir_all(projects.join(slug)).unwrap();
            let file = std::fs::File::create(projects.join(slug).join("s1.jsonl")).unwrap();
            file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(age)).unwrap();
        }
        std::fs::create_dir_all(projects.join("-other")).unwrap();
        std::fs::write(projects.join("-other/s2.jsonl"), "").unwrap();
        assert_eq!(transcript(&home, "s1"), Some(projects.join("-new/s1.jsonl")));
        assert_eq!(transcript(&home, "s2"), Some(projects.join("-other/s2.jsonl")));
        assert_eq!(transcript(&home, "s3"), None);
        assert_eq!(transcript(&home.join("nowhere"), "s1"), None);
        let _ = std::fs::remove_dir_all(home);
    }
}
