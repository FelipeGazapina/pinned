//! The machine side of the Reviewer: U4's signed-out store and U8's review run. U9's send-back
//! is in `send_back.rs`.
//!
//! When a worker thread's turn would end its task and the project's review switch is on, a
//! read-only reviewer thread under the worker reads the task's change against the rules. The task
//! waits for it, then ends with the worker's latest reply and the round on its record. A review
//! that cannot finish never holds the task: it ends, and its notice says the review did not run.

use super::{captain_scope, git_remote, repo_key, Machine, UNWATCHED};
use crate::agent::{role_env, role_of, AgentEvent, AgentStatus, Role};
use crate::pinned_lines;
use crate::review_run::{self, Change};
use crate::reviewer::{ReviewRound, ReviewState, ReviewerPatch, Rule};
use crate::taste::{self, ModelRef, Tastes};
use crate::{accounts::Provider, history, workspace};
use std::path::Path;

/// A review that holds its task's end.
pub(super) struct LiveReview {
    reviewer: String,
    worker: String,
    model: String,
    note: Option<String>,
    rules: Vec<Rule>,
}

impl Machine {
    /// `Tastes::set_reviewer` on this machine's own tastes, for a signed-out app.
    pub fn set_local_reviewer(&self, patch: ReviewerPatch) -> Result<Tastes, String> {
        taste::change(self.orchestration_dir.as_deref(), |tastes| tastes.set_reviewer(patch))
    }

    /// U8's hook: the worker thread in `tab` running task `task` finished a turn that the task
    /// would end with. True when a review now holds the task's end: the worker has no origin, the
    /// project's switch is on with a rule, and the change is not empty. The worker's notice then
    /// waits for the review, as it does for a later turn end while the review runs. False lets the
    /// task end now, with `ended_by` saying so when a review was due but could not start.
    pub(super) fn review_turn_end(&self, task: &str, tab: &str) -> bool {
        if self.reviews.lock().unwrap().contains_key(task) {
            return true;
        }
        let Ok((Some((Role::Worker, project)), false)) = self.tab(tab, |t| (role_of(&t.env), t.origin.is_some())) else { return false };
        let Some(info) = self.delegates.get(task) else { return false };
        let tastes = self.tastes();
        let repo = git_remote(&info.cwd).and_then(|url| repo_key(&url));
        let Some(rules) = review_run::rules_for(&tastes.reviewer, repo.as_deref()) else { return false };
        let change = match self.read_change(&info.cwd, info.base.as_deref()) {
            Ok(change) if change.is_empty() => return false,
            Ok(change) => change,
            Err(()) => return self.review_did_not_run(task, "its change could not be read"),
        };
        let taste = tastes.resolve(&captain_scope(&history::project_of(&info.cwd)));
        let Some(wanted) = tastes.reviewer.model.clone().or_else(|| review_run::ladder_pick(&taste)) else { return false };
        // The model set, else the ladder's other rungs from the top down.
        let rungs = std::iter::once(wanted.clone()).chain(taste.ladder.iter().rev().filter(|rung| rung.model != wanted.model).cloned());
        let Some((model, account)) = rungs.into_iter().find_map(|rung| self.reviewer_account(&rung).map(|account| (rung, account))) else {
            return self.review_did_not_run(task, &format!("no account here can run {}", wanted.model));
        };
        let note = (model.model != wanted.model).then(|| format!("{} cannot run on this machine's accounts; the review ran on {}", wanted.model, model.model));
        let title = format!("Review: {}", info.title);
        let launch = vec!["--name".to_owned(), title.clone(), review_run::brief(&info.title, &rules, &change)];
        let id = self.add_tab(info.cwd.clone(), account, launch, role_env(Role::Reviewer, &project), false, None);
        let _ = self.tab(&id, |t| {
            t.make_read_only();
            t.title = Some(title.clone());
            t.model = Some(model.model.clone());
            t.spawned_by = Some(tab.to_owned());
        });
        if let Err(error) = self.start_claude(&id, UNWATCHED.0, UNWATCHED.1) {
            self.close_tab(&id);
            return self.review_did_not_run(task, &format!("the reviewer did not start: {}", pinned_lines::field(&error)));
        }
        // A new session clears the title, so it is set again.
        let _ = self.tab(&id, |t| t.title = Some(title));
        let live = LiveReview { reviewer: id, worker: tab.to_owned(), model: model.model, note, rules };
        self.reviews.lock().unwrap().insert(task.to_owned(), live);
        self.delegates.set_reviewing(task, true);
        self.arm_delegate_pass();
        self.emit_tabs();
        true
    }

    /// The task's change in its folder: what changed since the commit it started from, and the new
    /// files git does not track.
    fn read_change(&self, cwd: &Path, base: Option<&str>) -> Result<Change, ()> {
        let git = |args: &[&str]| workspace::run(cwd, self.launch.path_env.as_deref(), "git", args).map_err(|_| ());
        let base = base.unwrap_or("HEAD");
        Ok(Change {
            stat: git(&["diff", "--stat", base, "--"])?,
            diff: git(&["diff", base, "--"])?,
            untracked: git(&["ls-files", "--others", "--exclude-standard"])?.lines().map(str::to_owned).collect(),
        })
    }

    /// The first Claude account here that runs `rung` and no limit blocks.
    fn reviewer_account(&self, rung: &ModelRef) -> Option<crate::accounts::Account> {
        let open = |account: &crate::accounts::Account| !matches!(self.admit_account(account), crate::delegate::Admit::Blocked(_));
        self.accounts.list().into_iter().find(|account| account.provider == Provider::Claude && self.rung_runs_on(rung, account) && open(account))
    }

    /// Notes on task `task` that its review did not run, and why. Returns false: the task ends now.
    pub(super) fn review_did_not_run(&self, task: &str, reason: &str) -> bool {
        self.delegates.set_ended_by(task, format!("its review did not run: {reason}"));
        false
    }

    /// A reviewer thread's turn ended: its findings, or why it has none, go on the task it
    /// reviewed, which then ends. A reviewer whose review is gone, as after a restart, is ignored.
    pub(super) fn reviewer_turn_ended(&self, tab: &str, event: &AgentEvent) {
        let live = {
            let mut reviews = self.reviews.lock().unwrap();
            let task = reviews.iter().find(|(_, live)| live.reviewer == tab).map(|(task, _)| task.clone());
            task.and_then(|task| reviews.remove(&task).map(|live| (task, live)))
        };
        let Some((task, live)) = live else { return };
        let read = match event.status {
            AgentStatus::Failed => Err(format!("the reviewer's turn failed: {}", pinned_lines::field(event.message.as_deref().unwrap_or("the turn failed")))),
            _ => review_run::findings(&self.reply_of(tab, event).unwrap_or_default(), &live.rules).map_err(|_| "the reviewer's reply could not be read".to_owned()),
        };
        match read {
            Ok(findings) => {
                let state = if findings.is_empty() { ReviewState::Passed } else { ReviewState::Limit };
                self.delegates.put_review(&task, ReviewRound { round: 1, findings, state, model: Some(live.model), note: live.note });
            }
            Err(reason) => {
                self.review_did_not_run(&task, &reason);
            }
        }
        self.finish_review(&task, &live.worker);
    }

    /// Ends task `task` of the worker thread in `worker` once its review is over, with the worker's
    /// latest reply, as its turn end would have.
    pub(super) fn finish_review(&self, task: &str, worker: &str) {
        self.delegates.set_reviewing(task, false);
        let Some(info) = self.delegates.get(task).filter(|info| !info.state.terminal()) else { return };
        if !self.finish_thread_task(task, worker, info.summary.clone(), None, None) {
            if let Ok(Some((Role::Worker, project))) = self.tab(worker, |t| role_of(&t.env)) {
                let event = AgentEvent { status: AgentStatus::Done, cwd: None, message: None, tab: Some(worker.to_owned()), session: None, title: None, resume_at: None, prompt: None, snapshot: None, permission_mode: None, hook_feedback: false, reply: info.summary };
                self.report_worker(worker, &project, &event, Some(AgentStatus::Done));
            }
        }
        self.emit_tabs();
    }

    /// The delegate pass's step for live reviews: one whose task ended (cancelled, its time limit,
    /// its worker closed) closes its reviewer, and one whose reviewer was closed ends its task.
    pub(super) fn end_reviews_of_ended_tasks(&self) {
        let live: Vec<(String, String)> = self.reviews.lock().unwrap().iter().map(|(task, live)| (task.clone(), live.reviewer.clone())).collect();
        for (task, reviewer) in live {
            let ended = self.delegates.get(&task).is_none_or(|info| info.state.terminal());
            if !ended && self.tabs.lock().unwrap().contains_key(&reviewer) {
                continue;
            }
            let Some(live) = self.reviews.lock().unwrap().remove(&task) else { continue };
            if ended {
                self.delegates.set_reviewing(&task, false);
                self.close_tab(&reviewer);
            } else {
                self.review_did_not_run(&task, "the reviewer was closed");
                self.finish_review(&task, &live.worker);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::launch;
    use super::*;

    #[test]
    fn a_task_with_no_live_review_and_no_worker_tab_is_not_held() {
        let m = Machine::new(launch(), None, None, |_| {});
        assert!(!m.review_turn_end("t1", "tab-1"));
    }

    #[cfg(unix)]
    mod run {
        use super::super::super::tests::{delegate_task, git, launch, limited_captain, temp, wait_for, work};
        use super::*;
        use crate::accounts::Account;
        use crate::delegate::{Caller, Claim, Mode, NewTask, Parent, Thread};
        use crate::reviewer::Switch;
        use crate::taste::TastePatch;
        use serde_json::{json, Value};
        use std::path::PathBuf;
        use std::sync::Arc;

        const REPO: &str = "github.com/acme/site";

        /// A Captain of a project at `github.com/acme/site` whose review switch is `switch`, with
        /// the account rules `rules`, and a worker thread on a task there. Returns the machine,
        /// its folder, the Captain, the task and the worker's tab.
        fn worker_task(name: &str, switch: Switch, rules: &[&str]) -> (Arc<Machine>, PathBuf, String, String, String) {
            let (m, dir, captain) = limited_captain(name, TastePatch::default());
            // The worker's turn end prices its usage, which would download the rates.
            std::fs::write(dir.join("config/usage-model-rates.json"), "{}").unwrap();
            let app = dir.join("app");
            git(&app, &["init", "-q"]);
            git(&app, &["remote", "add", "origin", &format!("https://{REPO}.git")]);
            std::fs::write(app.join("a.rs"), "fn a() {}\n").unwrap();
            git(&app, &["add", "a.rs"]);
            git(&app, &["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "-m", "first"]);
            // The built-in account's home is the real ~/.claude: a limit keeps the reviewer off it.
            m.delegates.limits().mark("claude", "You've hit your monthly spend limit", Some(super::super::super::unix_now() + 7 * 86_400));
            set_review(&m, switch, rules);
            let started = delegate_task(&m, &captain, work(None, None));
            let (task, worker) = (started["taskId"].as_str().unwrap().to_owned(), started["tabId"].as_str().unwrap().to_owned());
            (m, dir, captain, task, worker)
        }

        fn set_review(m: &Machine, switch: Switch, rules: &[&str]) {
            let rules = rules.iter().enumerate().map(|(n, text)| Rule { id: format!("A{}", n + 1), text: (*text).into(), replaces: None }).collect();
            m.set_local_reviewer(ReviewerPatch { rules: Some(rules), ..ReviewerPatch::default() }).unwrap();
            m.set_local_reviewer(ReviewerPatch { project: Some(REPO.into()), switch: Some(switch), ..ReviewerPatch::default() }).unwrap();
        }

        fn change(dir: &Path) {
            std::fs::write(dir.join("app/a.rs"), "fn a() { b().unwrap(); }\n").unwrap();
        }

        fn done(tab: &str, reply: &str) -> AgentEvent {
            AgentEvent { status: AgentStatus::Done, cwd: None, message: None, tab: Some(tab.into()), session: None, title: None, resume_at: None, prompt: None, snapshot: None, permission_mode: None, hook_feedback: false, reply: Some(reply.into()) }
        }

        fn reviewer_tab(m: &Machine) -> Option<String> {
            m.tabs.lock().unwrap().iter().find(|(_, t)| matches!(role_of(&t.env), Some((Role::Reviewer, _)))).map(|(id, _)| id.clone())
        }

        /// `task_status` as the Captain asks it, which takes the notice of an ended task: read the notice first.
        fn status(m: &Machine, captain: &str, task: &str) -> Value {
            m.delegates.status(&Caller { tab: Some(captain.into()), task: None }, task).unwrap()
        }

        fn notice(m: &Machine, captain: &str) -> String {
            m.delegates.claim_notice(captain).map(|(text, _)| text).unwrap_or_default()
        }

        /// The worker's turn ended with "Fixed it." and a review started. Returns the reviewer's tab.
        fn under_review(m: &Machine, worker: &str) -> String {
            m.record_event(done(worker, "Fixed it."));
            reviewer_tab(m).expect("a reviewer started")
        }

        const ONE_FINDING: &str = "One thing.\n```json\n{\"findings\":[{\"rule\":\"A1\",\"file\":\"a.rs\",\"lines\":[1],\"what\":\"unwrap on b\"}]}\n```";

        #[test]
        fn review_turn_end_starts_nothing_unless_the_switch_is_on_with_a_rule_for_a_worker_with_a_change() {
            let (m, dir, _, task, worker) = worker_task("review-none", Switch::On, &["No unwrap."]);
            let tabs = || m.tabs.lock().unwrap().len();
            let held = |tab: &str| {
                let before = tabs();
                let held = m.review_turn_end(&task, tab);
                assert_eq!(tabs(), before, "a tab opened");
                held
            };
            assert!(!held(&worker), "an empty change");
            assert_eq!(m.delegates.get(&task).unwrap().ended_by, None, "an empty change says nothing");
            change(&dir);
            for switch in [Switch::Auto, Switch::Off] {
                set_review(&m, switch, &["No unwrap."]);
                assert!(!held(&worker), "{switch:?}");
            }
            set_review(&m, Switch::On, &[]);
            assert!(!held(&worker), "no rules");
            set_review(&m, Switch::On, &["No unwrap."]);
            let app = dir.join("app");
            let other = m.add_tab(app.clone(), Account::builtin(Provider::Claude), vec![], role_env(Role::Reviewer, &app), false, None);
            assert!(!held(&other), "a reviewer's turn");
            let origin = crate::delegate::Origin { machine: "m-2".into(), machine_name: String::new(), tab: "tab-9".into(), task: "t-9".into(), access: crate::delegate::Access::Auto, delegator: Default::default(), limits: Default::default() };
            m.tab(&worker, |t| t.origin = Some(origin)).unwrap();
            assert!(!held(&worker), "a worker for another machine");
            m.tab(&worker, |t| t.origin = None).unwrap();
            assert_eq!(reviewer_tab(&m), Some(other), "only the stand-in is a reviewer");
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_worker_with_a_change_is_held_by_a_read_only_reviewer_whose_findings_reach_the_notice() {
            let (m, dir, captain, task, worker) = worker_task("review-found", Switch::On, &["No unwrap in the hub."]);
            change(&dir);
            let reviewer = under_review(&m, &worker);
            let (env, read_only, spawned_by, has_task) = m.tab(&reviewer, |t| (t.env.clone(), t.read_only, t.spawned_by.clone(), t.task.is_some())).unwrap();
            assert!(env.contains(&("PINNED_ROLE".into(), "reviewer".into())), "{env:?}");
            assert_eq!((read_only, spawned_by.as_deref(), has_task), (true, Some(worker.as_str()), false));
            let args = dir.join(format!("claude.{reviewer}"));
            wait_for("the reviewer's start", || args.exists());
            let args = std::fs::read_to_string(args).unwrap();
            let data = args.split("<change>\n").nth(1).unwrap().split("\n</change>").next().unwrap();
            assert!(args.contains("A1: No unwrap in the hub.") && data.contains("a.rs") && data.contains("b().unwrap()"), "{args}");
            let working = status(&m, &captain, &task);
            assert_eq!(working["workState"], "working");
            assert!(m.delegates.get(&task).unwrap().reviewing);
            assert_eq!(m.delegates.claim_notice(&captain), None, "the notice waits for the review");

            m.record_event(done(&worker, "Fixed it, really."));
            assert_eq!(reviewer_tab(&m), Some(reviewer.clone()), "a later turn end starts no second review");
            m.record_event(done(&reviewer, ONE_FINDING));
            let line = notice(&m, &captain);
            assert!(line.contains(r#"Its review found 1 finding: A1 in "a.rs" lines 1: "unwrap on b"."#), "{line}");
            let ended = status(&m, &captain, &task);
            assert_eq!((ended["workState"].as_str(), ended["summary"].as_str()), (Some("result_available"), Some("Fixed it, really.")), "{ended}");
            assert_eq!(ended["reviews"], json!([{ "round": 1, "state": "limit", "model": "claude-sonnet-5-5", "findings": [{ "rule": "A1", "file": "a.rs", "lines": [1], "what": "unwrap on b" }] }]));
            assert!(!m.delegates.get(&task).unwrap().reviewing);
            assert!(m.tab(&reviewer, |_| ()).is_ok(), "the reviewer stays open");
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_passed_review_records_its_round_and_leaves_the_notice_as_it_was() {
            let (m, dir, captain, task, worker) = worker_task("review-passed", Switch::On, &["No unwrap."]);
            let nova = ModelRef { provider: None, model: "claude-nova-9-9".into() };
            m.set_local_reviewer(ReviewerPatch { model: Some(Some(nova)), ..ReviewerPatch::default() }).unwrap();
            change(&dir);
            let reviewer = under_review(&m, &worker);
            m.record_event(done(&reviewer, "```json\n{\"findings\":[]}\n```"));
            let line = notice(&m, &captain);
            let ended = status(&m, &captain, &task);
            let note = "claude-nova-9-9 cannot run on this machine's accounts; the review ran on claude-opus-5-5";
            assert_eq!(ended["reviews"], json!([{ "round": 1, "state": "passed", "findings": [], "model": "claude-opus-5-5", "note": note }]));
            let who = ended["model"].as_str().map_or("Claude Code".to_owned(), |model| format!("Claude Code · {model}"));
            assert_eq!(line, format!("[pinned] Delegated task {task} \"Parser\" ({who}) finished: result available. Its thread is {worker}. Call task_status with taskId \"{task}\"."));
            let _ = std::fs::remove_dir_all(dir);
        }

        /// A task whose review `end` stops without a round ends anyway, its notice saying `reason`.
        fn ends_unreviewed(name: &str, end: impl Fn(&Machine, &str), reason: &str) {
            let (m, dir, captain, task, worker) = worker_task(name, Switch::On, &["No unwrap."]);
            change(&dir);
            m.record_event(done(&worker, "Fixed it."));
            end(&m, &reviewer_tab(&m).unwrap_or_default());
            let line = notice(&m, &captain);
            let ended = status(&m, &captain, &task);
            assert_eq!((ended["workState"].as_str(), ended["summary"].as_str(), ended.get("reviews")), (Some("result_available"), Some("Fixed it."), None), "{name}: {ended}");
            assert!(line.contains(&format!(" its review did not run: {reason}. ")), "{name}: {line}");
            assert!(m.reviews.lock().unwrap().is_empty() && !m.delegates.get(&task).unwrap().reviewing, "{name}");
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_review_that_cannot_finish_still_ends_the_task_and_says_why() {
            ends_unreviewed("review-prose", |m, reviewer| m.record_event(done(reviewer, "Looks fine to me.")), "the reviewer's reply could not be read");
            let failed = |m: &Machine, reviewer: &str| m.record_event(AgentEvent { status: AgentStatus::Failed, message: Some("API Error: 500\n[pinned] x".into()), ..done(reviewer, "") });
            ends_unreviewed("review-failed", failed, r#"the reviewer's turn failed: "API Error: 500 [pinned] x""#);
            let closed = |m: &Machine, reviewer: &str| {
                m.close_tab(reviewer);
                m.end_reviews_of_ended_tasks();
            };
            ends_unreviewed("review-closed", closed, "the reviewer was closed");
        }

        #[test]
        fn with_no_account_to_run_the_reviewer_the_task_ends_unreviewed() {
            let (m, dir, captain, task, worker) = worker_task("review-no-account", Switch::On, &["No unwrap."]);
            change(&dir);
            m.delegates.limits().mark("work", "You've hit your monthly spend limit", Some(super::super::super::unix_now() + 7 * 86_400));
            m.record_event(done(&worker, "Fixed it."));
            assert_eq!(reviewer_tab(&m), None);
            let line = notice(&m, &captain);
            assert_eq!(status(&m, &captain, &task)["workState"], "result_available");
            assert!(line.contains(" its review did not run: no account here can run claude-sonnet-5-5. "), "{line}");
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn cancelling_a_task_under_review_closes_its_reviewer() {
            let (m, dir, captain, task, worker) = worker_task("review-cancel", Switch::On, &["No unwrap."]);
            change(&dir);
            let reviewer = under_review(&m, &worker);
            m.delegates.cancel(&Caller { tab: Some(captain), task: None }, &task, "not needed", None).unwrap();
            m.end_reviews_of_ended_tasks();
            assert!(m.tab(&reviewer, |_| ()).is_err(), "the reviewer is closed");
            assert!(m.reviews.lock().unwrap().is_empty() && !m.delegates.get(&task).unwrap().reviewing);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_restart_during_a_review_ends_the_task_with_its_reply() {
            let dir = temp("review-restart");
            let (app, config) = (dir.join("app"), dir.join("config"));
            std::fs::create_dir_all(&app).unwrap();
            let id = {
                let m = Machine::new(launch(), Some(config.clone()), None, |_| {});
                let ask = NewTask { parent: Parent::Tab("tab-old".into()), tool: None, title: "Fix".into(), account: "claude".into(), provider: Provider::Claude, model: None, cwd: app.clone(), mode: Mode::Async, access: crate::delegate::Access::Auto, isolated: false, caller_key: "tab:tab-old".into(), client_request_id: None, role: None };
                let Claim::Fresh(task) = m.delegates.claim(ask, None) else { panic!("the task was not new") };
                m.delegates.link_thread(&task.id, Thread { tab: "tab-gone".into(), runs: true });
                m.delegates.set_worker_session(&task.id, "s1");
                m.delegates.turn_finished(&task.id, Some("Fixed it.".into()), vec![]);
                m.delegates.set_reviewing(&task.id, true);
                task.id
            };
            let saved = json!([{ "cwd": app, "session": "s1", "env": role_env(Role::Worker, &app) }]);
            std::fs::write(config.join("sessions.json"), saved.to_string()).unwrap();
            let m = Machine::new(launch(), Some(config), None, |_| {});
            m.restore_tabs();
            let task = m.delegates.get(&id).unwrap();
            assert_eq!((task.state, task.summary.as_deref(), task.reviewing), (crate::delegate::WorkState::ResultAvailable, Some("Fixed it."), false));
            let line = notice(&m, "tab-old");
            assert!(line.contains(" its review did not run: pinned restarted during it. "), "{line}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
