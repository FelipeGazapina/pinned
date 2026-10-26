//! A workflow session's work as another machine needs it to resume: the conversation, and the
//! code with whatever was not pushed yet, uncommitted files included.

use crate::wire::Bytes;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    let _ = (home, session);
    todo!()
}

/// The folder the session last worked in, as its transcript records it.
pub fn session_cwd(transcript: &Path) -> Option<PathBuf> {
    let _ = transcript;
    todo!()
}

/// The work in `cwd`, or None outside a repository, before its first commit, or when the
/// bundle would pass `limit` bytes.
pub fn capture_code(cwd: &Path, limit: u64) -> Option<Code> {
    let _ = (cwd, limit);
    todo!()
}

/// Puts the checkpoint's work in the worktree `.claude/worktrees/<name>` of `checkout`, replacing
/// one an earlier attempt left, and its transcript where Claude Code looks for that folder's
/// sessions. Answers the folder to resume in.
pub fn restore(home: &Path, checkout: &Path, name: &str, checkpoint: &Checkpoint) -> Result<PathBuf, String> {
    let _ = (home, checkout, name, checkpoint);
    todo!()
}
