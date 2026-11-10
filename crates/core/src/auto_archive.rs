//! Archiving a finished session on its own. Finished worker threads pile up in the sidebar, so a
//! machine looks at its tabs now and then and asks the hub to archive the ones whose work is over
//! and whose report went up, once they have idled for a grace period. Archive, never delete: the
//! hub's `archive_session` keeps the conversation with its name, and Unarchive brings it back.
//!
//! The rule is two halves. `keeps` runs on the machine, which knows the tab's turns, tasks and
//! folder; `hub_keeps` runs on the hub, which knows the pins. Each returns what keeps the session
//! open, or None when it may go. When in doubt, it stays.

use crate::agent::{AgentStatus, Role};
use crate::delegate::ThreadEnd;
use crate::global_orchestrator::{offer_of, waiting_kind};
use crate::marks::SessionMarks;
use crate::wire::TabView;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One file in the machine's config folder.
pub const SETTINGS_FILE: &str = "auto-archive.json";

/// How often a machine looks at its tabs, in seconds.
pub const CHECK_SECS: u64 = 60;

/// How long a machine leaves a session alone after the hub or its folder kept it, in seconds.
pub const RETRY_SECS: u64 = 10 * 60;

/// The grace periods the settings page offers, in minutes. Any other value a file holds works too.
pub const GRACES: [u32; 4] = [10, 30, 120, 24 * 60];

/// When this machine archives a finished session. A grace period is in minutes, and null is never.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[schemars(rename = "AutoArchiveSettings")]
pub struct Settings {
    /// Worker threads, the tabs a `delegate_task` started.
    #[serde(default = "default_workers")]
    pub workers: Option<u32>,
    /// Every other session, the ones the user opened.
    #[serde(default)]
    pub sessions: Option<u32>,
}

fn default_workers() -> Option<u32> {
    Some(30)
}

impl Default for Settings {
    fn default() -> Self {
        Settings { workers: default_workers(), sessions: None }
    }
}

impl Settings {
    pub fn load(dir: Option<&Path>) -> Self {
        dir.and_then(|d| std::fs::read(d.join(SETTINGS_FILE)).ok()).and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok()).unwrap_or_default().cleaned()
    }

    pub fn save(&self, dir: Option<&Path>) -> Result<(), String> {
        let Some(dir) = dir else { return Ok(()) };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(SETTINGS_FILE), serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    /// A grace of 0 minutes is no grace at all, so it counts as one minute.
    pub fn cleaned(self) -> Self {
        let grace = |minutes: Option<u32>| minutes.map(|m| m.max(1));
        Settings { workers: grace(self.workers), sessions: grace(self.sessions) }
    }

    /// The grace period that applies to `tab`, in minutes.
    fn grace_of(&self, tab: &TabView) -> Option<u32> {
        if tab.role == Some(Role::Worker) {
            self.workers
        } else {
            self.sessions
        }
    }
}

/// What keeps a session open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keeps {
    /// The setting for this kind of session is off.
    Off,
    /// The session is the global orchestrator or a Capitão.
    Orchestrator,
    /// The user starred it.
    Pinned,
    /// It has no conversation yet, which the hub archives nothing of.
    NoConversation,
    /// A turn is running or about to, a message waits for it, or the user is typing into it.
    Busy,
    /// It waits on the user: a question, an approval, or a failure.
    Waiting,
    /// Its last turn offered something that the user may still take up.
    Offer,
    /// A task it ran has not ended, or its result has not reached the session that delegated it.
    Task,
    /// A task it started still works.
    Child,
    /// It has idled for less than the grace period, or nothing says since when.
    Grace,
    /// Its folder holds uncommitted changes.
    Dirty,
    /// Its folder holds commits that no remote has.
    Ahead,
    /// Git would not say what its folder holds.
    Unknown,
}

impl std::fmt::Display for Keeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Keeps::Off => "auto-archive is off for it",
            Keeps::Orchestrator => "an orchestrator is never archived on its own",
            Keeps::Pinned => "it is pinned",
            Keeps::NoConversation => "it has no conversation yet",
            Keeps::Busy => "it is not idle",
            Keeps::Waiting => "it waits on the user",
            Keeps::Offer => "it has an open offer",
            Keeps::Task => "a task it ran has not ended or not been delivered",
            Keeps::Child => "a task it started still works",
            Keeps::Grace => "it has not idled long enough",
            Keeps::Dirty => "its folder has uncommitted changes",
            Keeps::Ahead => "its folder has commits no remote has",
            Keeps::Unknown => "git would not say what its folder holds",
        })
    }
}

/// What the machine knows of one tab.
pub struct Facts<'a> {
    pub tab: &'a TabView,
    /// The session is between turns: none runs or waits to start, and the user types nothing.
    pub idle: bool,
    /// Each task whose thread is this tab.
    pub tasks: &'a [ThreadEnd],
    /// How many tasks that this tab started still work.
    pub children: usize,
    /// Unix seconds.
    pub now: u64,
}

/// Why the machine keeps `facts.tab`, or None when only its folder, which `unpushed` reads, and the
/// hub's pins stand between it and the archive.
pub fn keeps(facts: &Facts, settings: &Settings) -> Option<Keeps> {
    let tab = facts.tab;
    if tab.orchestrator_global || tab.role == Some(Role::Orchestrator) {
        return Some(Keeps::Orchestrator);
    }
    let Some(grace) = settings.grace_of(tab) else { return Some(Keeps::Off) };
    if tab.session.is_none() {
        return Some(Keeps::NoConversation);
    }
    if waiting_kind(tab).is_some() {
        return Some(Keeps::Waiting);
    }
    if offer_of(tab, facts.now).is_some() {
        return Some(Keeps::Offer);
    }
    if !facts.idle || tab.status == Some(AgentStatus::Working) || tab.interrupted || tab.queued > 0 {
        return Some(Keeps::Busy);
    }
    if facts.tasks.iter().any(|task| !task.terminal || !task.delivered) {
        return Some(Keeps::Task);
    }
    if facts.children > 0 {
        return Some(Keeps::Child);
    }
    let turn_ended = tab.updated.or(tab.last_message_at);
    let delivered = facts.tasks.iter().map(|task| task.at).max();
    let Some(since) = turn_ended.into_iter().chain(delivered).max() else { return Some(Keeps::Grace) };
    (facts.now.saturating_sub(since) < u64::from(grace) * 60).then_some(Keeps::Grace)
}

/// Why the hub keeps `tab`, whose marks are `marks`, or None when it may archive it. The hub asks
/// again what the machine asked, since a machine's word is not the account's.
pub fn hub_keeps(tab: &TabView, marks: &SessionMarks) -> Option<Keeps> {
    if tab.orchestrator_global || tab.role == Some(Role::Orchestrator) {
        return Some(Keeps::Orchestrator);
    }
    if marks.pin.is_some() {
        return Some(Keeps::Pinned);
    }
    if tab.status == Some(AgentStatus::Working) {
        return Some(Keeps::Busy);
    }
    None
}

/// Whether `folder` holds work that exists nowhere else: uncommitted changes, or commits that no
/// remote has. None when it holds none, or when it is no git work tree. A git that fails keeps it.
pub fn unpushed(folder: &Path, path: Option<&str>) -> Option<Keeps> {
    use crate::workspace::{run, Failure};
    let git = |args: &[&str]| run(folder, path, "git", args);
    match git(&["rev-parse", "--is-inside-work-tree"]) {
        Ok(inside) if inside.trim() == "true" => {}
        Ok(_) | Err(Failure::Said(_)) => return None,
        Err(Failure::Missing) => return Some(Keeps::Unknown),
    }
    let Ok(status) = git(&["status", "--porcelain"]) else { return Some(Keeps::Unknown) };
    if !status.trim().is_empty() {
        return Some(Keeps::Dirty);
    }
    let ahead = match git(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]) {
        Ok(upstream) => git(&["rev-list", "--count", &format!("{}..HEAD", upstream.trim())]),
        Err(_) => git(&["rev-list", "--count", "HEAD", "--not", "--remotes"]),
    };
    match ahead.ok().and_then(|count| count.trim().parse::<u64>().ok()) {
        Some(0) => None,
        Some(_) => Some(Keeps::Ahead),
        None => Some(Keeps::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::TabView;

    const NOW: u64 = 1_800_000_000;

    fn tab() -> TabView {
        serde_json::from_value(serde_json::json!({
            "id": "tab-1", "cwd": "/tmp/work", "bypass": false, "running": true, "status": "done",
            "title": null, "message": null, "resume_at": null, "connector": null, "opened": 1,
            "updated": NOW - 40 * 60, "session": "s1", "role": "worker",
        }))
        .unwrap()
    }

    fn delivered(at: u64) -> ThreadEnd {
        ThreadEnd { terminal: true, delivered: true, at }
    }

    fn verdict(tab: &TabView, tasks: &[ThreadEnd], children: usize, settings: &Settings) -> Option<Keeps> {
        keeps(&Facts { tab, idle: true, tasks, children, now: NOW }, settings)
    }

    #[test]
    fn a_finished_worker_whose_report_was_delivered_goes_after_the_grace_period() {
        let tasks = [delivered(NOW - 35 * 60)];
        assert_eq!(verdict(&tab(), &tasks, 0, &Settings::default()), None);
    }

    #[test]
    fn the_grace_period_counts_from_the_later_of_the_last_turn_and_the_delivery() {
        let settings = Settings::default();
        let delivered_late = [delivered(NOW - 5 * 60)];
        assert_eq!(verdict(&tab(), &delivered_late, 0, &settings), Some(Keeps::Grace), "delivered 5 minutes ago, turn ended 40");
        let mut turn_late = tab();
        turn_late.updated = Some(NOW - 29 * 60);
        assert_eq!(verdict(&turn_late, &[delivered(NOW - 90 * 60)], 0, &settings), Some(Keeps::Grace), "turn ended 29 minutes ago");
        turn_late.updated = Some(NOW - 30 * 60);
        assert_eq!(verdict(&turn_late, &[delivered(NOW - 90 * 60)], 0, &settings), None, "exactly 30 minutes");
    }

    #[test]
    fn a_session_that_says_nothing_of_when_it_ended_stays() {
        let mut silent = tab();
        silent.updated = None;
        assert_eq!(verdict(&silent, &[], 0, &Settings::default()), Some(Keeps::Grace));
        silent.last_message_at = Some(NOW - 3600);
        assert_eq!(verdict(&silent, &[], 0, &Settings::default()), None, "its last message dates it");
    }

    #[test]
    fn a_task_that_has_not_ended_or_not_been_delivered_keeps_the_thread() {
        let settings = Settings::default();
        assert_eq!(verdict(&tab(), &[ThreadEnd { terminal: false, delivered: false, at: 0 }], 0, &settings), Some(Keeps::Task));
        assert_eq!(verdict(&tab(), &[ThreadEnd { terminal: true, delivered: false, at: NOW - 3600 }], 0, &settings), Some(Keeps::Task));
        assert_eq!(verdict(&tab(), &[delivered(NOW - 3600), ThreadEnd { terminal: false, delivered: false, at: 0 }], 0, &settings), Some(Keeps::Task), "a reopened thread's second task counts");
    }

    #[test]
    fn a_running_child_task_keeps_the_thread() {
        assert_eq!(verdict(&tab(), &[delivered(NOW - 3600)], 1, &Settings::default()), Some(Keeps::Child));
    }

    #[test]
    fn a_busy_session_stays() {
        let settings = Settings::default();
        let idle = |tab: &TabView, idle: bool| keeps(&Facts { tab, idle, tasks: &[], children: 0, now: NOW }, &settings);
        assert_eq!(idle(&tab(), false), Some(Keeps::Busy), "a turn runs or waits to start");
        let mut working = tab();
        working.status = Some(AgentStatus::Working);
        assert_eq!(idle(&working, true), Some(Keeps::Busy));
        let mut interrupted = tab();
        interrupted.interrupted = true;
        assert_eq!(idle(&interrupted, true), Some(Keeps::Busy), "a turn was cut off by a restart");
        let mut queued = tab();
        queued.queued = 2;
        assert_eq!(idle(&queued, true), Some(Keeps::Busy), "messages wait for it");
    }

    #[test]
    fn a_pending_ask_or_a_failure_keeps_the_session() {
        let settings = Settings::default();
        let mut asking = tab();
        asking.status = Some(AgentStatus::NeedsInput);
        asking.interrupted = true;
        asking.message = Some("Delete it?".into());
        assert_eq!(waiting_kind(&asking), Some(crate::global_orchestrator::WaitingKind::NeedsInput));
        assert_eq!(verdict(&asking, &[], 0, &settings), Some(Keeps::Waiting), "a question pending mid-turn");
        let mut failed = tab();
        failed.status = Some(AgentStatus::Failed);
        assert_eq!(verdict(&failed, &[], 0, &settings), Some(Keeps::Waiting), "a failure waits for a look");
    }

    #[test]
    fn an_offer_keeps_a_session_the_user_opened_for_a_day() {
        let settings = Settings { workers: Some(30), sessions: Some(30) };
        let mut offering = tab();
        offering.role = None;
        offering.question = Some("Want me to open a PR?".into());
        assert_eq!(verdict(&offering, &[], 0, &settings), Some(Keeps::Offer));
        offering.updated = Some(NOW - crate::global_orchestrator::OFFER_SECS - 1);
        assert_eq!(verdict(&offering, &[], 0, &settings), None, "an offer older than a day has dropped");
    }

    #[test]
    fn the_setting_decides_which_kind_of_session_goes() {
        let user = {
            let mut tab = tab();
            tab.role = None;
            tab
        };
        let delivered = [delivered(NOW - 3600)];
        let defaults = Settings::default();
        assert_eq!(verdict(&tab(), &delivered, 0, &defaults), None, "workers are on by default");
        assert_eq!(verdict(&user, &[], 0, &defaults), Some(Keeps::Off), "sessions the user opened are off by default");
        assert_eq!(verdict(&user, &[], 0, &Settings { workers: None, sessions: Some(120) }), Some(Keeps::Grace), "two hours, idle for 40 minutes");
        assert_eq!(verdict(&user, &[], 0, &Settings { workers: None, sessions: Some(10) }), None);
        assert_eq!(verdict(&tab(), &delivered, 0, &Settings { workers: None, sessions: Some(10) }), Some(Keeps::Off), "never, for workers");
    }

    #[test]
    fn an_orchestrator_is_never_archived() {
        let settings = Settings { workers: Some(1), sessions: Some(1) };
        let mut captain = tab();
        captain.role = Some(Role::Orchestrator);
        assert_eq!(verdict(&captain, &[], 0, &settings), Some(Keeps::Orchestrator));
        let mut global = tab();
        global.orchestrator_global = true;
        assert_eq!(verdict(&global, &[], 0, &settings), Some(Keeps::Orchestrator));
        assert_eq!(hub_keeps(&captain, &SessionMarks::new("s1")), Some(Keeps::Orchestrator));
        assert_eq!(hub_keeps(&global, &SessionMarks::new("s1")), Some(Keeps::Orchestrator));
    }

    #[test]
    fn a_session_without_a_conversation_stays() {
        let mut fresh = tab();
        fresh.session = None;
        assert_eq!(verdict(&fresh, &[], 0, &Settings::default()), Some(Keeps::NoConversation));
    }

    #[test]
    fn the_hub_keeps_a_starred_thread_and_a_working_one() {
        let mut starred = SessionMarks::new("s1");
        starred.pin = Some(0);
        assert_eq!(hub_keeps(&tab(), &starred), Some(Keeps::Pinned));
        assert_eq!(hub_keeps(&tab(), &SessionMarks::new("s1")), None);
        let mut working = tab();
        working.status = Some(AgentStatus::Working);
        assert_eq!(hub_keeps(&working, &SessionMarks::new("s1")), Some(Keeps::Busy));
    }

    #[test]
    fn settings_default_to_workers_after_half_an_hour_and_nothing_for_the_rest() {
        assert_eq!(Settings::default(), Settings { workers: Some(30), sessions: None });
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(Some(dir.path())), Settings::default());
        std::fs::write(dir.path().join(SETTINGS_FILE), r#"{"workers":null,"sessions":120}"#).unwrap();
        assert_eq!(Settings::load(Some(dir.path())), Settings { workers: None, sessions: Some(120) }, "a null is never");
        std::fs::write(dir.path().join(SETTINGS_FILE), r#"{"sessions":0}"#).unwrap();
        assert_eq!(Settings::load(Some(dir.path())), Settings { workers: Some(30), sessions: Some(1) }, "a missing field keeps its default, and 0 minutes counts as 1");
        std::fs::write(dir.path().join(SETTINGS_FILE), "not json").unwrap();
        assert_eq!(Settings::load(Some(dir.path())), Settings::default());
        let saved = Settings { workers: Some(10), sessions: None };
        saved.save(Some(dir.path())).unwrap();
        assert_eq!(Settings::load(Some(dir.path())), saved);
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = crate::process::command("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn commit(dir: &Path, message: &str) {
        git(dir, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", message]);
    }

    /// A clone of a repository whose first commit is pushed, so its branch tracks `origin/main`.
    fn clone_with_upstream(root: &Path) -> std::path::PathBuf {
        let origin = root.join("origin.git");
        git(root, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
        let seed = root.join("seed");
        std::fs::create_dir(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        commit(&seed, "first");
        git(&seed, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git(&seed, &["push", "-q", "origin", "main"]);
        let work = root.join("work");
        git(root, &["clone", "-q", origin.to_str().unwrap(), work.to_str().unwrap()]);
        work
    }

    #[test]
    fn a_folder_with_nothing_unpushed_may_go() {
        let root = tempfile::tempdir().unwrap();
        let work = clone_with_upstream(root.path());
        assert_eq!(unpushed(&work, None), None);
        let plain = root.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert_eq!(unpushed(&plain, None), None, "a folder that is no git work tree has nothing to lose");
    }

    #[test]
    fn uncommitted_changes_keep_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let work = clone_with_upstream(root.path());
        std::fs::write(work.join("notes.txt"), "draft").unwrap();
        assert_eq!(unpushed(&work, None), Some(Keeps::Dirty), "an untracked file");
        git(&work, &["add", "notes.txt"]);
        assert_eq!(unpushed(&work, None), Some(Keeps::Dirty), "a staged file");
    }

    #[test]
    fn commits_ahead_of_the_upstream_keep_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let work = clone_with_upstream(root.path());
        commit(&work, "second");
        assert_eq!(unpushed(&work, None), Some(Keeps::Ahead));
        git(&work, &["push", "-q", "origin", "main"]);
        assert_eq!(unpushed(&work, None), None, "once pushed it may go");
    }

    #[test]
    fn a_branch_with_no_upstream_keeps_its_local_commits() {
        let root = tempfile::tempdir().unwrap();
        let work = clone_with_upstream(root.path());
        git(&work, &["switch", "-q", "-c", "fix-it"]);
        assert_eq!(unpushed(&work, None), None, "a new branch with no commit of its own");
        commit(&work, "mine");
        assert_eq!(unpushed(&work, None), Some(Keeps::Ahead), "no upstream, one local commit");
        git(&work, &["push", "-q", "origin", "fix-it"]);
        assert_eq!(unpushed(&work, None), None, "a remote branch has the commit now");
    }

    #[test]
    fn a_linked_worktree_is_read_as_its_own_folder() {
        let root = tempfile::tempdir().unwrap();
        let work = clone_with_upstream(root.path());
        let tree = work.join(".claude/worktrees/task");
        git(&work, &["worktree", "add", "-q", "-b", "task", tree.to_str().unwrap()]);
        assert_eq!(unpushed(&tree, None), None);
        std::fs::write(tree.join("a.txt"), "x").unwrap();
        assert_eq!(unpushed(&tree, None), Some(Keeps::Dirty));
        assert_eq!(unpushed(&work, None), None, "the main checkout is untouched");
    }

    #[test]
    fn a_repository_whose_git_says_nothing_keeps_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let fresh = root.path().join("fresh");
        std::fs::create_dir(&fresh).unwrap();
        git(&fresh, &["init", "-q", "-b", "main"]);
        assert_eq!(unpushed(&fresh, None), Some(Keeps::Unknown), "a repository with no commit has no history to compare");
        assert_eq!(unpushed(&fresh, Some("/nonexistent")), Some(Keeps::Unknown), "no git on the path");
    }
}
