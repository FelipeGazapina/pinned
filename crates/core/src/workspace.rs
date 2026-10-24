use crate::history;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};

const DIFF_LIMIT: usize = 400_000;
const PR_FIELDS: &str = "number,title,state,isDraft,url,body,author,createdAt,headRefName,baseRefName,additions,deletions,changedFiles,reviewDecision,mergeable,statusCheckRollup,reviews,comments";

#[derive(Serialize, Debug, PartialEq)]
pub struct Workspace {
    /// The folder the project's history is kept under, which a worktree shares with its repository.
    pub project: String,
    /// The folder is inside a git repository.
    pub repo: bool,
    pub branch: Option<String>,
    pub worktree: bool,
}

pub fn of(cwd: &Path) -> Workspace {
    let git = |args: &[&str]| run(cwd, None, "git", args).ok().map(|out| out.trim().to_owned()).filter(|out| !out.is_empty());
    // symbolic-ref names the branch of a repository with no commit yet too; a detached HEAD has none.
    let branch = git(&["symbolic-ref", "--short", "-q", "HEAD"]);
    let dir = git(&["rev-parse", "--git-dir"]);
    let worktree = matches!((&dir, git(&["rev-parse", "--git-common-dir"])), (Some(dir), Some(common)) if *dir != common);
    Workspace { project: history::project_of(cwd).display().to_string(), repo: dir.is_some(), branch, worktree }
}

/// What `gh` knows of the pull request `pr`, a number or URL, with its diff under `diff`. `path` is
/// the PATH to find gh on.
pub fn pull_request(cwd: &Path, pr: &str, path: Option<&str>) -> Result<Value, String> {
    let is_number = !pr.is_empty() && pr.bytes().all(|b| b.is_ascii_digit());
    if !is_number && !(pr.starts_with("https://") && pr.contains("/pull/")) {
        return Err(format!("{pr:?} is not a pull request number or URL"));
    }
    let gh = |args: &[&str]| {
        run(cwd, path, "gh", args).map_err(|e| match e {
            Failure::Missing => "the GitHub CLI isn't installed on this machine. Install gh and run `gh auth login`.".to_owned(),
            Failure::Said(said) => format!("gh: {said}"),
        })
    };
    let view = gh(&["pr", "view", "--json", PR_FIELDS, "--", pr])?;
    let mut view: Value = serde_json::from_str(&view).map_err(|e| format!("gh sent a bad answer: {e}"))?;
    let mut diff = gh(&["pr", "diff", "--", pr])?;
    if diff.len() > DIFF_LIMIT {
        let cut = (0..=DIFF_LIMIT).rev().find(|i| diff.is_char_boundary(*i)).unwrap_or(0);
        diff.truncate(cut);
        view["truncated"] = true.into();
    }
    view["diff"] = diff.into();
    Ok(view)
}

enum Failure {
    Missing,
    Said(String),
}

fn run(cwd: &Path, path: Option<&str>, program: &str, args: &[&str]) -> Result<String, Failure> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd).stdin(Stdio::null());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let out = command.output().map_err(|_| Failure::Missing)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(Failure::Said(String::from_utf8_lossy(&out.stderr).trim().to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        assert!(Command::new("git").args(args).current_dir(dir).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success(), "git {args:?}");
    }

    #[test]
    fn a_worktree_reports_its_branch_and_its_repository_as_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "first"]);
        let tree = repo.join(".claude/worktrees/fix");
        git(&repo, &["worktree", "add", "-q", "-b", "fix-it", tree.to_str().unwrap()]);

        let fresh = repo.join("fresh");
        std::fs::create_dir(&fresh).unwrap();
        git(&fresh, &["init", "-q", "-b", "trunk"]);
        assert_eq!((of(&fresh).repo, of(&fresh).branch.as_deref()), (true, Some("trunk")), "a repository with no commit yet");

        let main = of(&repo);
        assert_eq!((main.repo, main.branch.as_deref(), main.worktree), (true, Some("main"), false));
        let wt = of(&tree);
        assert_eq!((wt.branch.as_deref(), wt.worktree, wt.project), (Some("fix-it"), true, repo.display().to_string()));
    }

    #[test]
    fn a_pull_request_must_be_a_number_or_url() {
        let dir = tempfile::tempdir().unwrap();
        for pr in ["--web", "-R other/repo", "", "12a", "http://github.com/o/r/pull/1"] {
            assert!(pull_request(dir.path(), pr, None).unwrap_err().contains("is not a pull request"), "{pr:?}");
        }
        let missing = pull_request(dir.path(), "7", Some(dir.path().to_str().unwrap())).unwrap_err();
        assert!(missing.contains("isn't installed"), "{missing}");
    }

    #[test]
    fn a_plain_folder_has_no_branch() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(of(dir.path()), Workspace { project: dir.path().display().to_string(), repo: false, branch: None, worktree: false });
    }
}
