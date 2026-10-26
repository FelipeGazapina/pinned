use crate::history;
use crate::workspace::{run, Failure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A folder's branches and where its own stands, for the composer's branch picker and Git action.
#[derive(Serialize, Debug, PartialEq)]
pub struct Branches {
    /// Local branches, then remote ones with no local branch of that name, newest commit first.
    pub refs: Vec<Ref>,
    /// The working tree has changes, staged or not.
    pub dirty: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// The remote's default branch, such as `main`.
    pub default: Option<String>,
    /// The repository has an `origin` to push to.
    pub remote: bool,
    /// HEAD is on no branch.
    pub detached: bool,
    /// Commits on HEAD that the remote's default branch lacks.
    pub ahead_of_default: Option<u32>,
    /// The changed files, relative to the repository's top, as git status lists them.
    pub changed: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Ref {
    /// `fix-it`, or `origin/fix-it` for a remote one.
    pub name: String,
    pub current: bool,
    pub remote: bool,
    /// Checked out in another worktree, so git refuses to switch to it here.
    pub worktree: bool,
}

pub fn of(cwd: &Path, path: Option<&str>) -> Result<Branches, String> {
    let git = |args: &[&str]| run(cwd, path, "git", args).map_err(said);
    let listed = git(&["for-each-ref", "--sort=-committerdate", "--format=%(HEAD)%00%(refname)%00%(worktreepath)", "refs/heads", "refs/remotes"])?;
    let mut refs: Vec<Ref> = vec![];
    for line in listed.lines() {
        let mut parts = line.split('\0');
        let (Some(head), Some(name), worktree) = (parts.next(), parts.next(), parts.next().unwrap_or("")) else { continue };
        let current = head == "*";
        if let Some(local) = name.strip_prefix("refs/heads/") {
            refs.push(Ref { name: local.into(), current, remote: false, worktree: !current && !worktree.is_empty() });
        } else if let Some(remote) = name.strip_prefix("refs/remotes/").filter(|r| !r.ends_with("/HEAD")) {
            refs.push(Ref { name: remote.into(), current: false, remote: true, worktree: false });
        }
    }
    let locals: Vec<String> = refs.iter().filter(|r| !r.remote).map(|r| r.name.clone()).collect();
    refs.retain(|r| !r.remote || !r.name.split_once('/').is_some_and(|(_, n)| locals.iter().any(|l| l == n)));
    refs.sort_by_key(|r| (!r.current, r.remote));

    let status = git(&["status", "--porcelain=v2", "--branch"])?;
    let mut out = Branches { refs, dirty: false, upstream: None, ahead: 0, behind: 0, default: None, remote: false, detached: false, ahead_of_default: None, changed: vec![] };
    for line in status.lines() {
        if line == "# branch.head (detached)" {
            out.detached = true;
        }
        // A branch with no commit yet has no ref to list.
        if let Some(head) = line.strip_prefix("# branch.head ").filter(|h| *h != "(detached)" && !out.refs.iter().any(|r| r.current)) {
            out.refs.insert(0, Ref { name: head.into(), current: true, remote: false, worktree: false });
        } else if let Some(upstream) = line.strip_prefix("# branch.upstream ") {
            out.upstream = Some(upstream.into());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            let mut counts = ab.split(' ').map(|n| n.trim_start_matches(['+', '-']).parse().unwrap_or(0));
            (out.ahead, out.behind) = (counts.next().unwrap_or(0), counts.next().unwrap_or(0));
        } else if !line.starts_with('#') {
            out.dirty = true;
        }
    }
    out.default = git(&["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"]).ok().and_then(|r| r.trim().split_once('/').map(|(_, b)| b.to_owned()));
    out.remote = git(&["remote", "get-url", "origin"]).is_ok();
    // A branch with no upstream is ahead by the commits no remote branch has.
    if out.upstream.is_none() {
        out.ahead = git(&["rev-list", "--count", "HEAD", "--not", "--remotes"]).ok().and_then(|n| n.trim().parse().ok()).unwrap_or(0);
    }
    out.ahead_of_default = out.default.as_ref().and_then(|d| git(&["rev-list", "--count", &format!("origin/{d}..HEAD")]).ok()).and_then(|n| n.trim().parse().ok());
    out.changed = changed(cwd, path)?;
    Ok(out)
}

/// The changed files of `cwd`'s working tree, renamed ones by their new name.
fn changed(cwd: &Path, path: Option<&str>) -> Result<Vec<String>, String> {
    let listed = run(cwd, path, "git", &["status", "--porcelain=v1", "-z", "-uall"]).map_err(said)?;
    let mut entries = listed.split('\0').filter(|e| !e.is_empty());
    let mut files = vec![];
    while let Some(entry) = entries.next() {
        if entry.len() > 3 {
            files.push(entry[3..].to_owned());
        }
        if entry.starts_with('R') || entry.starts_with('C') {
            entries.next();
        }
    }
    Ok(files)
}

/// Switches `cwd` to the local branch `branch`, made from the remote one of that name if there is
/// one, or a new branch at HEAD when `create`. With `from`, `branch` is made or reset to start at
/// that ref instead, fetched first when it is a remote one, as a new worktree picks its base.
pub fn checkout(cwd: &Path, branch: &str, create: bool, from: Option<&str>, path: Option<&str>) -> Result<(), String> {
    if branch.starts_with('-') || run(cwd, path, "git", &["check-ref-format", "--branch", branch]).is_err() {
        return Err(format!("{branch:?} is not a branch name"));
    }
    if let Some(from) = from {
        if from.starts_with('-') {
            return Err(format!("{from:?} is not a ref"));
        }
        if let Some((remote, name)) = from.split_once('/').filter(|(remote, _)| *remote == "origin") {
            run(cwd, path, "git", &["fetch", "-q", remote, name]).map_err(said)?;
        }
        return run(cwd, path, "git", &["switch", "-q", "-C", branch, from]).map(|_| ()).map_err(said);
    }
    let args: &[&str] = if create { &["switch", "-c", branch] } else { &["switch", branch] };
    run(cwd, path, "git", args).map(|_| ()).map_err(said)
}

/// A new worktree of the repository holding `cwd`, on a new branch `pinned/<token>` from what `cwd`
/// has checked out, in the `.claude/worktrees` folder claude's own --worktree uses, which git is
/// told to leave out of the repository's status.
pub fn new_worktree(cwd: &Path, path: Option<&str>) -> Result<PathBuf, String> {
    let git = |args: &[&str]| run(cwd, path, "git", args).map(|out| out.trim().to_owned()).map_err(said);
    let root = history::project_of(Path::new(&git(&["rev-parse", "--show-toplevel"])?));
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let token = format!("{:08x}", (nanos as u64 ^ (u64::from(std::process::id()) << 20)) as u32);
    let dir = root.join(".claude/worktrees").join(format!("pinned-{token}"));
    let exclude = cwd.join(git(&["rev-parse", "--git-common-dir"])?).join("info/exclude");
    let listed = std::fs::read_to_string(&exclude).unwrap_or_default();
    if !listed.lines().any(|l| l.trim() == "/.claude/worktrees/") {
        let _ = std::fs::create_dir_all(exclude.parent().unwrap_or(cwd));
        let _ = std::fs::write(&exclude, format!("{listed}{}/.claude/worktrees/\n", if listed.is_empty() || listed.ends_with('\n') { "" } else { "\n" }));
    }
    git(&["worktree", "add", "-q", "-b", &format!("pinned/{token}"), &dir.display().to_string(), "HEAD"])?;
    Ok(dir)
}

/// Removes a worktree `new_worktree` made, and its branch, while nothing was done in them: git
/// keeps a worktree with changes and a branch with commits of its own.
pub fn drop_worktree(dir: &Path, path: Option<&str>) {
    let branch = run(dir, path, "git", &["symbolic-ref", "--short", "-q", "HEAD"]).map(|b| b.trim().to_owned()).unwrap_or_default();
    let root = history::project_of(dir);
    if run(&root, path, "git", &["worktree", "remove", &dir.display().to_string()]).is_ok() && branch.starts_with("pinned/") {
        let _ = run(&root, path, "git", &["branch", "-d", &branch]);
    }
}

/// One step of the header's Git action, which the page runs in order to show where it is.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum GitStep {
    /// A commit message for `files`, or every change, written by a headless claude.
    Message {
        #[serde(default)]
        files: Vec<String>,
    },
    /// Commits `files`, or every change, with `message`.
    Commit {
        message: String,
        #[serde(default)]
        files: Vec<String>,
    },
    /// Pushes the branch, setting its upstream on `origin` the first time.
    Push,
    /// Opens a pull request of the branch with gh, titled and described from its commits.
    CreatePr,
    /// Fast-forwards the branch to its upstream.
    Pull,
    /// Makes the folder a git repository.
    Init,
    /// Creates `name` on GitHub with gh, as `origin`, and pushes to it.
    Publish { name: String, public: bool },
}

/// Runs `step` in `cwd`: what it answers is the commit message, the pull request's URL, or `null`.
/// `claude` is the binary that writes a commit message.
pub fn run_step(cwd: &Path, step: &GitStep, claude: &Path, path: Option<&str>) -> Result<Option<String>, String> {
    let git = |args: &[&str]| run(cwd, path, "git", args).map_err(said);
    let stage = |files: &[String]| -> Result<(), String> {
        if files.iter().any(|f| f.starts_with('-')) {
            return Err("a file name can't start with -".into());
        }
        let mut args = vec!["add", "-A", "--"];
        args.extend(files.iter().map(String::as_str));
        git(&args).map(|_| ())
    };
    match step {
        GitStep::Message { files } => {
            stage(files)?;
            let stat = git(&["diff", "--cached", "--stat"])?;
            let mut diff = git(&["diff", "--cached"])?;
            diff.truncate((0..=diff.len().min(20_000)).rev().find(|i| diff.is_char_boundary(*i)).unwrap_or(0));
            let ask = format!("Write the git commit message for the change below. Answer with the message alone: an imperative subject of at most 72 characters, then a blank line and a short body only if it helps.\n\n{stat}\n{diff}");
            let mut command = Command::new(claude);
            command.args(["-p", "--model", "haiku", &ask]).current_dir(cwd).stdin(Stdio::null());
            if let Some(path) = path {
                command.env("PATH", path);
            }
            for key in ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_CHILD_SESSION"] {
                command.env_remove(key);
            }
            let written = command.output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).filter(|m| !m.is_empty());
            let count = stat.lines().count().saturating_sub(1).max(1);
            Ok(Some(written.unwrap_or_else(|| format!("Update {count} file{}", if count == 1 { "" } else { "s" }))))
        }
        GitStep::Commit { message, files } => {
            stage(files)?;
            let mut args = vec!["commit", "-q", "-m", message.as_str()];
            if !files.is_empty() {
                args.push("--");
                args.extend(files.iter().map(String::as_str));
            }
            git(&args).map(|_| None)
        }
        GitStep::Push => git(&["push", "-q", "-u", "origin", "HEAD"]).map(|_| None),
        GitStep::CreatePr => run(cwd, path, "gh", &["pr", "create", "--fill"]).map(|out| out.lines().rev().find(|l| l.starts_with("https://")).map(str::to_owned)).map_err(said),
        GitStep::Pull => git(&["pull", "-q", "--ff-only"]).map(|_| None),
        GitStep::Init => git(&["init", "-q"]).map(|_| None),
        GitStep::Publish { name, public } => {
            if name.starts_with('-') || name.is_empty() {
                return Err(format!("{name:?} is not a repository name"));
            }
            run(cwd, path, "gh", &["repo", "create", name, if *public { "--public" } else { "--private" }, "--source", ".", "--remote", "origin", "--push"]).map(|_| None).map_err(said)
        }
    }
}

fn said(e: Failure) -> String {
    match e {
        Failure::Missing => "git isn't installed on this machine".into(),
        Failure::Said(said) => said.strip_prefix("fatal: ").unwrap_or(&said).to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").args(["-c", "user.email=a@b", "-c", "user.name=a"]).args(args).current_dir(dir).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn a_repository_with_no_commit_lists_its_branch() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "trunk"]);
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let b = of(dir.path(), None).unwrap();
        assert_eq!((b.refs, b.dirty), (vec![Ref { name: "trunk".into(), current: true, remote: false, worktree: false }], true));
    }

    #[test]
    fn lists_branches_with_the_current_first_and_switches_between_them() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&origin).unwrap();
        git(&origin, &["init", "-q", "-b", "main"]);
        git(&origin, &["commit", "-q", "--allow-empty", "-m", "first"]);
        git(&origin, &["branch", "shared"]);
        git(&origin, &["branch", "only-remote"]);
        git(dir.path(), &["clone", "-q", origin.to_str().unwrap(), "repo"]);
        git(&repo, &["branch", "shared", "origin/shared"]);
        git(&repo, &["switch", "-q", "-c", "feature"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "second"]);
        std::fs::write(repo.join("new.txt"), "x").unwrap();
        git(&repo, &["worktree", "add", "-q", dir.path().join("wt").to_str().unwrap(), "shared"]);

        let b = of(&repo, None).unwrap();
        let names: Vec<_> = b.refs.iter().map(|r| (r.name.as_str(), r.current, r.remote, r.worktree)).collect();
        assert_eq!(names[0], ("feature", true, false, false));
        assert!(names.contains(&("shared", false, false, true)), "{names:?}");
        assert!(names.contains(&("origin/only-remote", false, true, false)), "{names:?}");
        assert!(!names.iter().any(|n| n.0 == "origin/shared" || n.0 == "origin/main"), "a remote with a local twin: {names:?}");
        assert_eq!((b.dirty, b.upstream.as_deref(), b.default.as_deref()), (true, None, Some("main")));

        checkout(&repo, "main", false, None, None).unwrap();
        assert_eq!(of(&repo, None).unwrap().refs[0].name, "main");
        checkout(&repo, "only-remote", false, None, None).unwrap();
        let b = of(&repo, None).unwrap();
        assert_eq!((b.refs[0].name.as_str(), b.upstream.as_deref()), ("only-remote", Some("origin/only-remote")));
        checkout(&repo, "brand-new", true, None, None).unwrap();
        assert_eq!(of(&repo, None).unwrap().refs[0].name, "brand-new");
        assert!(checkout(&repo, "shared", false, None, None).unwrap_err().contains("already"), "a branch another worktree has");
        for bad in ["--orphan", "a..b", ""] {
            assert!(checkout(&repo, bad, false, None, None).unwrap_err().contains("not a branch name"), "{bad:?}");
        }
    }

    #[test]
    fn a_new_worktree_branches_off_the_checkout_out_of_the_repositorys_status_and_goes_when_unused() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
        git(&repo, &["switch", "-q", "-c", "feature"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "second"]);

        let tree = new_worktree(&repo, None).unwrap();
        let tree = std::fs::canonicalize(&tree).unwrap();
        assert_eq!(tree.parent().unwrap(), std::fs::canonicalize(&repo).unwrap().join(".claude/worktrees"));
        let branch = of(&tree, None).unwrap().refs[0].name.clone();
        assert!(branch.starts_with("pinned/"), "{branch}");
        let head = |d: &Path| run(d, None, "git", &["rev-parse", "HEAD"]).ok().unwrap();
        assert_eq!(head(&tree), head(&repo), "it starts from what the checkout has");
        assert!(!of(&repo, None).unwrap().dirty, "the worktree folder shows in the repository's status");
        let again = new_worktree(&tree, None).unwrap();
        assert_eq!(std::fs::canonicalize(again.parent().unwrap()).unwrap(), tree.parent().unwrap(), "a worktree's worktree goes beside it");

        checkout(&tree, &branch, true, Some("main"), None).unwrap();
        assert_eq!((of(&tree, None).unwrap().refs[0].name.clone(), head(&tree)), (branch.clone(), run(&repo, None, "git", &["rev-parse", "main"]).ok().unwrap()), "a base resets the branch");

        drop_worktree(&tree, None);
        assert!(!tree.exists());
        assert!(!of(&repo, None).unwrap().refs.iter().any(|r| r.name == branch), "the unused branch stays");
        std::fs::write(again.join("work.txt"), "x").unwrap();
        drop_worktree(&again, None);
        assert!(again.join("work.txt").exists(), "a worktree with changes went");
    }

    #[test]
    fn git_steps_commit_chosen_files_with_a_written_message_and_push() {
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("remote.git");
        git(dir.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        run_step(&repo, &GitStep::Init, Path::new("/bin/false"), None).unwrap();
        git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        std::fs::write(repo.join("b.txt"), "b").unwrap();
        assert_eq!(of(&repo, None).unwrap().changed, ["a.txt", "b.txt"]);

        let claude = dir.path().join("claude");
        std::fs::write(&claude, "#!/bin/sh\necho 'Add the a file'\n").unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let files = vec!["a.txt".to_owned()];
        let message = run_step(&repo, &GitStep::Message { files: files.clone() }, &claude, None).unwrap().unwrap();
        assert_eq!(message, "Add the a file");
        assert_eq!(run_step(&repo, &GitStep::Message { files: vec![] }, Path::new("/bin/false"), None).unwrap().unwrap(), "Update 2 files", "with no claude");
        git(&repo, &["reset", "-q"]);
        run_step(&repo, &GitStep::Commit { message, files }, &claude, None).unwrap();
        let b = of(&repo, None).unwrap();
        assert_eq!((b.changed, b.remote, b.detached), (vec!["b.txt".to_owned()], true, false), "b stayed out of the commit");
        run_step(&repo, &GitStep::Push, &claude, None).unwrap();
        let b = of(&repo, None).unwrap();
        assert_eq!((b.upstream.as_deref(), b.ahead), (Some("origin/main"), 0));
        assert!(run_step(&repo, &GitStep::Commit { message: "x".into(), files: vec!["--all".into()] }, &claude, None).is_err());
    }
}
