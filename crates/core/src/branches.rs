use crate::workspace::{run, Failure};
use serde::Serialize;
use std::path::Path;

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
    let mut out = Branches { refs, dirty: false, upstream: None, ahead: 0, behind: 0, default: None };
    for line in status.lines() {
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
    Ok(out)
}

/// Switches `cwd` to the local branch `branch`, made from the remote one of that name if there is
/// one, or a new branch at HEAD when `create`.
pub fn checkout(cwd: &Path, branch: &str, create: bool, path: Option<&str>) -> Result<(), String> {
    if branch.starts_with('-') || run(cwd, path, "git", &["check-ref-format", "--branch", branch]).is_err() {
        return Err(format!("{branch:?} is not a branch name"));
    }
    let args: &[&str] = if create { &["switch", "-c", branch] } else { &["switch", branch] };
    run(cwd, path, "git", args).map(|_| ()).map_err(said)
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

        checkout(&repo, "main", false, None).unwrap();
        assert_eq!(of(&repo, None).unwrap().refs[0].name, "main");
        checkout(&repo, "only-remote", false, None).unwrap();
        let b = of(&repo, None).unwrap();
        assert_eq!((b.refs[0].name.as_str(), b.upstream.as_deref()), ("only-remote", Some("origin/only-remote")));
        checkout(&repo, "brand-new", true, None).unwrap();
        assert_eq!(of(&repo, None).unwrap().refs[0].name, "brand-new");
        assert!(checkout(&repo, "shared", false, None).unwrap_err().contains("already"), "a branch another worktree has");
        for bad in ["--orphan", "a..b", ""] {
            assert!(checkout(&repo, bad, false, None).unwrap_err().contains("not a branch name"), "{bad:?}");
        }
    }
}
