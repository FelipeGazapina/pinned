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
/// Where the work a restore replaced stays, as `<REPLACED>/<worktree name>`.
const REPLACED: &str = "refs/pinned/replaced";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// Claude Code's id for the conversation.
    pub session: String,
    /// The session's transcript, as Claude Code wrote it, gzipped.
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

/// Everything another machine needs to resume `session`, or None while it has no transcript.
/// The code goes along while its bundle stays under `limit` bytes.
pub fn capture(home: &Path, session: &str, limit: u64) -> Option<Checkpoint> {
    let file = transcript(home, session)?;
    let raw = std::fs::read(&file).ok()?;
    let cwd = session_cwd(&file);
    let worktree = cwd.as_deref().and_then(worktree_name);
    let code = cwd.as_deref().filter(|cwd| cwd.is_dir()).and_then(|cwd| capture_code(cwd, limit));
    Some(Checkpoint { session: session.to_owned(), transcript: Bytes(crate::sync::pack(&raw)), worktree, code })
}

/// The name of the worktree under `.claude/worktrees` that `cwd` is in, if any.
fn worktree_name(cwd: &Path) -> Option<String> {
    let parts: Vec<Component> = cwd.components().collect();
    parts.windows(3).find_map(|w| match w {
        [Component::Normal(a), Component::Normal(b), Component::Normal(name)] if *a == ".claude" && *b == "worktrees" => {
            Some(name.to_string_lossy().into_owned())
        }
        _ => None,
    })
}

fn gunzip(gz: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut raw = vec![];
    flate2::read::GzDecoder::new(gz).read_to_end(&mut raw).map_err(|e| format!("the checkpoint's transcript is corrupt: {e}"))?;
    Ok(raw)
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
    let commit = snapshot_commit(cwd)?;
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

/// A commit of `cwd`'s whole working tree on top of HEAD, leaving the tree and index as they are.
fn snapshot_commit(cwd: &Path) -> Option<String> {
    let head = git(cwd, &["rev-parse", "--verify", "-q", "HEAD"]).ok()?;
    let tree = crate::history::snapshot(cwd)?;
    git_with(cwd, &["commit-tree", "--no-gpg-sign", &tree, "-p", &head, "-m", "pinned checkpoint"], &IDENTITY).ok()
}

/// The folder a run resumes `session` in: its worktree `.claude/worktrees/<name>` of `checkout`,
/// or `checkout` itself when there is none, with the conversation where Claude Code looks from
/// there. This machine's own worktree is kept when its transcript already holds everything
/// `checkpoint`'s does, as on the machine that captured it; otherwise the checkpoint replaces it.
pub fn resume_in(home: &Path, checkout: &Path, name: &str, session: Option<&str>, checkpoint: Option<&Checkpoint>) -> Result<PathBuf, String> {
    let target = checkout.join(".claude/worktrees").join(plain(name)?);
    let cwd = match checkpoint {
        Some(checkpoint) if !holds(home, &target, checkpoint) => restore(home, checkout, name, checkpoint)?,
        _ if target.is_dir() => target,
        _ => checkout.to_path_buf(),
    };
    if let Some(session) = session {
        reachable(home, &cwd, plain(session)?)?;
    }
    Ok(cwd)
}

/// Whether the worktree at `target` is at least as far along as `checkpoint`. Claude Code only
/// appends to a transcript, so a local one that starts with the checkpoint's has all of it.
fn holds(home: &Path, target: &Path, checkpoint: &Checkpoint) -> bool {
    let local = transcript(home, &checkpoint.session).and_then(|file| std::fs::read(file).ok());
    target.is_dir() && local.zip(gunzip(&checkpoint.transcript.0).ok()).is_some_and(|(local, saved)| local.starts_with(&saved))
}

/// Copies the newest transcript of `session` to where Claude Code looks for it from `cwd`.
fn reachable(home: &Path, cwd: &Path, session: &str) -> Result<(), String> {
    let Some(newest) = transcript(home, session) else { return Ok(()) };
    let dir = crate::sync::claude_dir(home, cwd);
    let file = dir.join(format!("{session}.jsonl"));
    if newest == file {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    std::fs::copy(&newest, &file).map(drop).map_err(|e| format!("could not copy the transcript: {e}"))
}

/// Puts the checkpoint's work in the worktree `.claude/worktrees/<name>` of `checkout`, and its
/// transcript where Claude Code looks for that folder's sessions. The work of a worktree it
/// replaces stays under `REPLACED`. Answers the folder to resume in.
fn restore(home: &Path, checkout: &Path, name: &str, checkpoint: &Checkpoint) -> Result<PathBuf, String> {
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
    if let Some(work) = target.is_dir().then(|| snapshot_commit(&target)).flatten() {
        git(checkout, &["update-ref", &format!("{REPLACED}/{name}"), &work])?;
    }
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
    std::fs::write(dir.join(format!("{session}.jsonl")), gunzip(&checkpoint.transcript.0)?).map_err(|e| format!("could not write the transcript: {e}"))?;
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
        // A session id becomes a `claude` argument, where a leading dash would read as a flag.
        (Some(Component::Normal(_)), None) if !name.starts_with('-') => Ok(name),
        _ => Err(format!("{name:?} is not a plain name")),
    }
}

fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    git_with(cwd, args, &[])
}

fn git_with(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .envs(env.iter().copied())
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
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
        Checkpoint { session: "s1".into(), transcript: Bytes(crate::sync::pack(b"{\"cwd\":\"/x\"}\n")), worktree: Some("w".into()), code }
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
        assert_eq!(run(&b, &["show", "refs/pinned/replaced/w:junk.txt"]), "left by the first attempt", "the replaced work stays in a ref");
        assert_eq!(run(&b, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_resume_keeps_a_worktree_that_is_further_along_than_the_checkpoint() {
        let root = temp("keep");
        let b = root.join("b");
        repo(&b);
        commit(&b, "tracked.txt", "v1\n", "init");
        let home = root.join("home");
        let cp = checkpoint(Some(capture_code(&b, 1 << 20).unwrap()));
        let target = resume_in(&home, &b, "w", Some("s1"), Some(&cp)).unwrap();
        let transcript = crate::sync::claude_dir(&home, &target).join("s1.jsonl");
        std::fs::write(target.join("tracked.txt"), "after the checkpoint\n").unwrap();
        std::fs::write(&transcript, "{\"cwd\":\"/x\"}\n{\"type\":\"assistant\"}\n").unwrap();
        assert_eq!(resume_in(&home, &b, "w", Some("s1"), Some(&cp)).unwrap(), target);
        assert_eq!(std::fs::read_to_string(target.join("tracked.txt")).unwrap(), "after the checkpoint\n");

        std::fs::write(&transcript, "{\"cwd\":\"/elsewhere\"}\n").unwrap();
        assert_eq!(resume_in(&home, &b, "w", Some("s1"), Some(&cp)).unwrap(), target);
        assert_eq!(std::fs::read_to_string(target.join("tracked.txt")).unwrap(), "v1\n", "a transcript that went another way is replaced");
        assert_eq!(run(&b, &["show", "refs/pinned/replaced/w:tracked.txt"]), "after the checkpoint");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_resume_without_a_checkpoint_goes_on_where_the_session_worked_and_finds_its_transcript() {
        let root = temp("local");
        let b = root.join("b");
        repo(&b);
        commit(&b, "tracked.txt", "v1\n", "init");
        let home = root.join("home");
        let started = crate::sync::claude_dir(&home, &b);
        std::fs::create_dir_all(&started).unwrap();
        std::fs::write(started.join("s1.jsonl"), "{}\n").unwrap();
        assert_eq!(resume_in(&home, &b, "w", Some("s1"), None).unwrap(), b);
        let target = b.join(".claude/worktrees/w");
        run(&b, &["worktree", "add", "-q", "--detach", target.to_str().unwrap()]);
        assert_eq!(resume_in(&home, &b, "w", Some("s1"), None).unwrap(), target);
        assert_eq!(std::fs::read_to_string(crate::sync::claude_dir(&home, &target).join("s1.jsonl")).unwrap(), "{}\n");
        assert_eq!(resume_in(&home, &b, "w", Some("--print"), None), Err("\"--print\" is not a plain name".into()));
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
    fn a_capture_finds_the_session_and_the_worktree_it_worked_in() {
        let home = temp("capture-home");
        let repo_dir = temp("capture-repo");
        repo(&repo_dir);
        commit(&repo_dir, "a.txt", "one\n", "init");
        let cwd = repo_dir.join(".claude/worktrees/eng-7");
        run(&repo_dir, &["worktree", "add", "-q", "-b", "eng-7", cwd.to_str().unwrap()]);
        std::fs::write(cwd.join("a.txt"), "two\n").unwrap();
        let line = format!("{{\"cwd\":{}}}\n", serde_json::to_string(cwd.to_str().unwrap()).unwrap());
        let dir = crate::sync::claude_dir(&home, &cwd);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("s9.jsonl"), &line).unwrap();

        let checkpoint = capture(&home, "s9", 1 << 20).unwrap();
        assert_eq!((checkpoint.session.as_str(), checkpoint.worktree.as_deref()), ("s9", Some("eng-7")));
        assert_eq!(gunzip(&checkpoint.transcript.0).unwrap(), line.as_bytes());
        assert_eq!(checkpoint.code.unwrap().branch.as_deref(), Some("eng-7"));
        assert_eq!(capture(&home, "missing", 1 << 20), None);
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(repo_dir);
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
