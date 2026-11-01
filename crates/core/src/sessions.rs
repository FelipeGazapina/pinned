//! Claude sessions still in progress when pinned quits, so the next launch resumes them.

use crate::accounts::{self, AccountId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, PartialEq, schemars::JsonSchema)]
pub struct Saved {
    pub cwd: PathBuf,
    /// Claude Code's session id, passed back as `claude --resume`.
    pub session: String,
    pub env: Vec<(String, String)>,
    /// The tab skips every permission check, and a resume must keep skipping them.
    #[serde(default)]
    pub bypass: bool,
    /// A turn was running (or stopped by an error) when pinned quit, so it has work left.
    #[serde(default)]
    pub interrupted: bool,
    /// Unix time a usage limit that stopped the turn lifts.
    #[serde(default)]
    pub resume_at: Option<u64>,
    /// The session waits for a prompt even once that limit lifts.
    #[serde(default)]
    pub held: bool,
    /// The session's transcript is in this account's home, so it resumes only there.
    #[serde(default = "accounts::default_account")]
    pub account: AccountId,
    /// The model picked for this conversation alone, passed as `claude --model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Claude's permission mode from the last hook that reported one. `plan` is read-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
}

/// What the harness types to a session that has work left.
pub const CONTINUE_PROMPT: &str = "Continue from where you left off.";

impl Saved {
    /// The `claude` arguments that reopen this session, and the usage limit still to wait for.
    /// An interrupted one picks its work back up at once, unless a limit still blocks it or it was
    /// held at one.
    pub fn reopen(&self, now: u64, provider: crate::accounts::Provider) -> (Vec<String>, Option<u64>) {
        let mut args = crate::delegate::resume_args(provider, &self.session);
        let waiting = self.resume_at.filter(|&at| self.interrupted && at > now);
        if self.interrupted && waiting.is_none() && !(self.held && self.resume_at.is_some()) {
            args.push(CONTINUE_PROMPT.to_owned());
        }
        (args, waiting)
    }
}

pub fn load(file: &Path) -> Vec<Saved> {
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(file: &Path, saved: &[Saved]) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // `load` reads a torn file as no sessions, so an exec in the middle of a write must find the old file whole.
    let tmp = file.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(saved).unwrap()).is_ok() {
        let _ = std::fs::rename(&tmp, file);
    }
}

/// The worktree a `claude --worktree` session in `folder` works in, given a cwd its hooks
/// reported. Resuming there keeps the session's edits off the main checkout.
pub fn worktree(folder: &Path, reported: &Path) -> Option<PathBuf> {
    let worktrees = folder.join(".claude/worktrees");
    let name = reported.strip_prefix(&worktrees).ok()?.components().next()?;
    Some(worktrees.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_is_found_from_any_folder_inside_it() {
        let repo = Path::new("/repo");
        assert_eq!(worktree(repo, Path::new("/repo/.claude/worktrees/eng-7/src/ui")), Some(PathBuf::from("/repo/.claude/worktrees/eng-7")));
        assert_eq!(worktree(repo, Path::new("/repo/src")), None);
        assert_eq!(worktree(repo, Path::new("/repo/.claude/worktrees")), None);
    }

    #[test]
    fn interrupted_sessions_continue_once_no_limit_blocks_them() {
        let saved = |interrupted, resume_at| Saved { cwd: "/repo".into(), session: "s1".into(), env: vec![], bypass: false, interrupted, resume_at, held: false, account: "claude".into(), model: None, permission_mode: None };
        let resumed = vec!["--resume".to_owned(), "s1".to_owned()];
        let continued = [resumed.clone(), vec![CONTINUE_PROMPT.to_owned()]].concat();
        let claude = crate::accounts::Provider::Claude;
        assert_eq!(saved(false, None).reopen(100, claude), (resumed.clone(), None));
        assert_eq!(saved(true, None).reopen(100, claude), (continued.clone(), None));
        assert_eq!(saved(true, Some(100)).reopen(100, claude), (continued, None));
        assert_eq!(saved(true, Some(101)).reopen(100, claude), (resumed.clone(), Some(101)));
        let held = Saved { held: true, ..saved(true, Some(100)) };
        assert_eq!(held.reopen(100, claude), (resumed, None), "a session held at its limit waits for a prompt");
        let codex = crate::accounts::Provider::Codex;
        assert_eq!(saved(false, None).reopen(100, codex), (vec!["resume".to_owned(), "s1".to_owned()], None));
        assert_eq!(saved(true, None).reopen(100, codex), (vec!["resume".to_owned(), "s1".to_owned(), CONTINUE_PROMPT.to_owned()], None));
    }

    #[test]
    fn saved_sessions_survive_a_round_trip() {
        let file = std::env::temp_dir().join(format!("pinned-sessions-test-{}.json", std::process::id()));
        let saved = vec![Saved {
            cwd: "/repo".into(),
            session: "23ec76b4-2867-49e2-b564-22c6c9e7b090".into(),
            env: vec![("PINNED_ISSUE".into(), "uuid".into())],
            bypass: true,
            interrupted: true,
            resume_at: Some(1792382400),
            held: true,
            account: "work".into(),
            model: Some("claude-opus-4-8".into()),
            permission_mode: Some("plan".into()),
        }];
        save(&file, &saved);
        assert_eq!(load(&file), saved);
        let _ = std::fs::remove_file(file);
        assert_eq!(load(Path::new("/nonexistent/sessions.json")), vec![]);
    }
}
