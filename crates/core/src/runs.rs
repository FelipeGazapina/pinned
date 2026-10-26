//! A workflow's runs: one per issue or Slack thread a connector started, kept in the connectors'
//! store from the check that found it until it ends, so a run outlives the tab and the machine
//! that ran it. Each check reconciles every run with the tabs the online machines show.

use crate::agent::{AgentStatus, Stage, Subject};
use crate::wire::{MachineId, TabId, TabView};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How long a run's machine may stay offline before another machine resumes the run.
pub const GRACE: u64 = 5 * 60;
/// Starts a run gets, its first included, before it stops and reports why: one per machine
/// lost under it and one per failed turn.
pub const ATTEMPTS: u32 = 3;
/// A run that ended this long ago leaves the store.
const KEEP: u64 = 30 * 24 * 3600;
const EVENTS: usize = 40;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Phase {
    /// Waits for a machine to take it, not before `after` (Unix seconds).
    Queued { after: u64 },
    /// Its session works on `machine`.
    Running { machine: MachineId },
    /// Its session finished its turn, or its tab closed, on `machine`. A Slack reply wakes it.
    Idle { machine: MachineId },
    /// Stopped after `ATTEMPTS` starts.
    Failed { error: String },
    Cancelled,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub at: u64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub connector: String,
    pub subject: Subject,
    /// What the runs list calls it, such as the issue identifier and title.
    pub title: String,
    #[serde(flatten)]
    pub phase: Phase,
    /// Starts so far: the first dispatch, each resume on another machine, each retried turn.
    pub attempt: u32,
    /// Claude Code's id for the conversation, which a resume continues.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The session's status as last seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
    /// Unix time a usage limit that stopped the session lifts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// When the failed turn last retried was reported, so a check never retries one turn twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retried: Option<u64>,
    /// The furthest stage the session reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    /// Hashes of the Slack replies posted, so a resumed session never posts one twice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<String>,
    /// Where and how the first dispatch started the session, which starts it over without a
    /// conversation to resume.
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub args: Vec<String>,
    /// The worktree a resume on another machine restores the work into.
    pub worktree: String,
    pub created: u64,
    pub updated: u64,
    /// Newest last.
    #[serde(default)]
    pub events: Vec<Event>,
}

/// A run as the runs list shows it, without the prompt it started with.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub key: String,
    pub connector: String,
    pub title: String,
    #[serde(flatten)]
    pub phase: Phase,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    pub created: u64,
    pub updated: u64,
    pub events: Vec<Event>,
}

/// The runs list: every run, the most recently changed first.
pub fn views<'a>(runs: impl IntoIterator<Item = (&'a String, &'a Job)>) -> Vec<View> {
    let mut views: Vec<View> = runs
        .into_iter()
        .map(|(key, j)| View {
            key: key.clone(),
            connector: j.connector.clone(),
            title: j.title.clone(),
            phase: j.phase.clone(),
            attempt: j.attempt,
            status: j.status,
            stage: j.stage,
            limit: j.limit,
            created: j.created,
            updated: j.updated,
            events: j.events.clone(),
        })
        .collect();
    views.sort_by_key(|v| std::cmp::Reverse(v.updated));
    views
}

/// What a run needs from the machines.
#[derive(Debug, PartialEq, Clone)]
pub enum Step {
    /// Start it on a machine: resume its conversation if it has one, else start over.
    Dispatch,
    /// Tell its session to go on after a failed turn.
    Continue,
    /// Close its tab on a machine that no longer holds the run.
    Close { machine: MachineId, tab: TabId },
    /// Tell the issue or the thread that the run stopped, and why.
    Report(String),
}

/// Every online machine with its tabs.
pub type Online = [(MachineId, Vec<TabView>)];

/// The store's key for the run of `connector` on `subject`.
pub fn key(connector: &str, subject: &Subject) -> String {
    match subject {
        Subject::LinearIssue { issue } => format!("{connector}/{issue}"),
        Subject::SlackThread { channel, thread } => format!("{connector}/{channel}:{thread}"),
    }
}

impl Job {
    pub fn key(&self) -> String {
        key(&self.connector, &self.subject)
    }

    /// The machine the run is on, while it holds one.
    pub fn machine(&self) -> Option<&MachineId> {
        match &self.phase {
            Phase::Running { machine } | Phase::Idle { machine } => Some(machine),
            _ => None,
        }
    }

    pub fn ended(&self) -> bool {
        matches!(self.phase, Phase::Failed { .. } | Phase::Cancelled)
    }

    /// Whether a run that ended, or went idle, long enough ago leaves the store.
    pub fn stale(&self, now: u64) -> bool {
        matches!(self.phase, Phase::Idle { .. } | Phase::Failed { .. } | Phase::Cancelled) && now.saturating_sub(self.updated) > KEEP
    }

    pub fn log(&mut self, now: u64, text: impl Into<String>) {
        self.events.push(Event { at: now, text: text.into() });
        let extra = self.events.len().saturating_sub(EVENTS);
        self.events.drain(..extra);
        self.updated = now;
    }

    fn set(&mut self, phase: Phase, now: u64, why: impl Into<String>) {
        self.phase = phase;
        self.log(now, why);
    }

    /// Records that `machine` took the run.
    pub fn dispatched(&mut self, machine: &MachineId, now: u64) {
        self.attempt += 1;
        let what = if self.attempt == 1 { "started".to_owned() } else { format!("attempt {} started", self.attempt) };
        self.set(Phase::Running { machine: machine.clone() }, now, format!("{what} on {machine}"));
    }

    pub fn fail(&mut self, error: String, now: u64) -> Step {
        self.set(Phase::Failed { error: error.clone() }, now, format!("stopped: {error}"));
        Step::Report(error)
    }

    /// Queues the run again from the start of its attempts.
    pub fn retry(&mut self, now: u64) {
        self.attempt = 0;
        self.retried = None;
        self.set(Phase::Queued { after: now }, now, "retried by hand");
    }

    pub fn cancel(&mut self, now: u64) {
        self.set(Phase::Cancelled, now, "cancelled");
    }

    /// Moves the run on from what the online machines show, and answers what it needs from them.
    /// `seen` is when its machine last showed it alive, or when the hub started watching, which
    /// gives every machine the same grace after a hub restart.
    pub fn reconcile(&mut self, online: &Online, seen: &mut u64, now: u64) -> Vec<Step> {
        let tabs: Vec<(&MachineId, &TabView)> = online
            .iter()
            .flat_map(|(machine, tabs)| tabs.iter().map(move |t| (machine, t)))
            .filter(|(_, t)| t.running && t.connector.as_deref() == Some(&self.connector) && t.subject.as_ref() == Some(&self.subject))
            .collect();
        let on = |machine: &MachineId| tabs.iter().find(|(m, _)| *m == machine).map(|(_, t)| *t);
        let mut steps = vec![];
        match self.phase.clone() {
            Phase::Queued { after } => match tabs.first() {
                // A machine already runs it, as when the hub restarted before the machines reconnected.
                Some((machine, tab)) => {
                    self.set(Phase::Running { machine: (*machine).clone() }, now, format!("found running on {machine}"));
                    *seen = now;
                    steps.extend(self.observe(tab, now));
                }
                None if now >= after => steps.push(Step::Dispatch),
                None => {}
            },
            Phase::Running { machine } if online.iter().any(|(m, _)| *m == machine) => {
                *seen = now;
                match on(&machine) {
                    Some(tab) => steps.extend(self.observe(tab, now)),
                    None => self.set(Phase::Idle { machine }, now, "its tab closed"),
                }
            }
            Phase::Running { machine } if now.saturating_sub(*seen) >= GRACE => match tabs.first() {
                Some((other, tab)) => {
                    self.set(Phase::Running { machine: (*other).clone() }, now, format!("found running on {other}"));
                    *seen = now;
                    steps.extend(self.observe(tab, now));
                }
                None if self.attempt >= ATTEMPTS => {
                    steps.push(self.fail(format!("{machine} went offline and the run has used its {ATTEMPTS} attempts"), now))
                }
                None => {
                    let after = self.limit.unwrap_or(now).max(now);
                    self.set(Phase::Queued { after }, now, format!("{machine} went offline"));
                    if after <= now {
                        steps.push(Step::Dispatch);
                    }
                }
            },
            Phase::Running { .. } => {}
            Phase::Idle { machine } => {
                let woken = tabs.iter().find(|(_, t)| matches!(t.status, Some(AgentStatus::Working | AgentStatus::NeedsInput)));
                if let Some((other, tab)) = woken {
                    self.set(Phase::Running { machine: (*other).clone() }, now, format!("working again on {other}"));
                    *seen = now;
                    steps.extend(self.observe(tab, now));
                } else if let Some(tab) = on(&machine) {
                    self.observe_session(tab);
                }
            }
            Phase::Failed { .. } | Phase::Cancelled => {}
        }
        let holder = self.machine().cloned();
        for (machine, tab) in &tabs {
            let stale = match (&self.phase, &holder) {
                (Phase::Cancelled, _) => true,
                (_, Some(holder)) => *machine != holder,
                _ => false,
            };
            if stale {
                steps.push(Step::Close { machine: (*machine).clone(), tab: tab.id.clone() });
            }
        }
        steps
    }

    fn observe_session(&mut self, tab: &TabView) {
        if tab.session.is_some() {
            self.session = tab.session.clone();
        }
        self.status = tab.status;
    }

    /// What the live tab of a running run says about it.
    fn observe(&mut self, tab: &TabView, now: u64) -> Vec<Step> {
        self.observe_session(tab);
        let Some(machine) = self.machine().cloned() else { return vec![] };
        match tab.status {
            Some(AgentStatus::Done) => {
                self.limit = None;
                self.set(Phase::Idle { machine }, now, "finished its turn");
                vec![]
            }
            // The machine continues the session itself once the limit lifts.
            Some(AgentStatus::Failed) if tab.resume_at.is_some() => {
                if self.limit != tab.resume_at {
                    self.limit = tab.resume_at;
                    self.log(now, "stopped by a usage limit");
                }
                vec![]
            }
            Some(AgentStatus::Failed) if self.retried != tab.updated => {
                self.retried = tab.updated;
                if self.attempt >= ATTEMPTS {
                    return vec![self.fail(format!("its turn failed and the run has used its {ATTEMPTS} attempts"), now)];
                }
                self.attempt += 1;
                self.log(now, format!("its turn failed; attempt {} continues it", self.attempt));
                vec![Step::Continue]
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn job(phase: Phase, attempt: u32) -> Job {
        Job {
            connector: "c1".into(),
            subject: Subject::LinearIssue { issue: "i1".into() },
            title: "ENG-1 Fix it".into(),
            phase,
            attempt,
            session: None,
            status: None,
            limit: None,
            retried: None,
            stage: None,
            replies: vec![],
            cwd: "/repo".into(),
            repo: None,
            args: vec!["go".into()],
            worktree: "eng-1".into(),
            created: NOW - 100,
            updated: NOW - 100,
            events: vec![],
        }
    }

    fn tab(id: &str, status: Option<AgentStatus>) -> TabView {
        TabView {
            id: id.into(),
            cwd: "/repo".into(),
            bypass: false,
            running: true,
            status,
            title: None,
            message: None,
            resume_at: None,
            connector: Some("c1".into()),
            subject: Some(Subject::LinearIssue { issue: "i1".into() }),
            opened: 1,
            updated: Some(NOW - 10),
            session: Some("s1".into()),
            stats: None,
        }
    }

    fn running(machine: &str) -> Phase {
        Phase::Running { machine: machine.into() }
    }

    #[test]
    fn a_queued_run_is_dispatched_once_its_time_comes() {
        let mut j = job(Phase::Queued { after: NOW + 60 }, 1);
        assert_eq!(j.reconcile(&[], &mut 0, NOW), vec![]);
        assert_eq!(j.reconcile(&[], &mut 0, NOW + 60), vec![Step::Dispatch]);
    }

    #[test]
    fn a_queued_run_a_machine_already_runs_is_adopted_not_dispatched_again() {
        let mut j = job(Phase::Queued { after: NOW }, 1);
        let online = [("m2".to_owned(), vec![tab("tab-3", Some(AgentStatus::Working))])];
        assert_eq!(j.reconcile(&online, &mut 0, NOW), vec![]);
        assert_eq!((j.phase, j.session.as_deref()), (running("m2"), Some("s1")));
    }

    #[test]
    fn a_run_whose_machine_is_offline_waits_out_the_grace_then_is_queued_and_dispatched() {
        let mut j = job(running("m1"), 1);
        let mut seen = NOW;
        let online = [("m2".to_owned(), vec![])];
        assert_eq!(j.reconcile(&online, &mut seen, NOW + GRACE - 1), vec![]);
        assert_eq!(j.phase, running("m1"));
        assert_eq!(j.reconcile(&online, &mut seen, NOW + GRACE), vec![Step::Dispatch]);
        assert_eq!(j.phase, Phase::Queued { after: NOW + GRACE });
        assert_eq!(j.events.last().unwrap().text, "m1 went offline");
    }

    #[test]
    fn a_run_stopped_by_a_usage_limit_on_a_lost_machine_waits_for_the_limit() {
        let mut j = job(running("m1"), 1);
        let mut seen = NOW;
        let mut limited = tab("tab-1", Some(AgentStatus::Failed));
        limited.resume_at = Some(NOW + 3600);
        j.reconcile(&[("m1".to_owned(), vec![limited])], &mut seen, NOW);
        assert_eq!((j.attempt, j.limit), (1, Some(NOW + 3600)));
        assert_eq!(j.reconcile(&[], &mut seen, NOW + GRACE), vec![]);
        assert_eq!(j.phase, Phase::Queued { after: NOW + 3600 });
        assert_eq!(j.reconcile(&[], &mut seen, NOW + 3600), vec![Step::Dispatch]);
    }

    #[test]
    fn a_machine_back_in_time_keeps_its_run() {
        let mut j = job(running("m1"), 1);
        let mut seen = NOW;
        assert_eq!(j.reconcile(&[], &mut seen, NOW + 60), vec![]);
        let online = [("m1".to_owned(), vec![tab("tab-1", Some(AgentStatus::Working))])];
        assert_eq!(j.reconcile(&online, &mut seen, NOW + GRACE + 1), vec![]);
        assert_eq!((j.phase, seen), (running("m1"), NOW + GRACE + 1));
    }

    #[test]
    fn a_run_that_lost_its_machine_too_often_fails_and_reports() {
        let mut j = job(running("m1"), ATTEMPTS);
        let steps = j.reconcile(&[], &mut NOW.clone(), NOW + GRACE);
        let error = "m1 went offline and the run has used its 3 attempts".to_owned();
        assert_eq!(steps, vec![Step::Report(error.clone())]);
        assert_eq!(j.phase, Phase::Failed { error });
    }

    #[test]
    fn the_old_machine_coming_back_has_its_tab_closed() {
        let mut j = job(running("m2"), 2);
        let online = [
            ("m1".to_owned(), vec![tab("tab-1", Some(AgentStatus::Working))]),
            ("m2".to_owned(), vec![tab("tab-7", Some(AgentStatus::Working))]),
        ];
        assert_eq!(j.reconcile(&online, &mut 0, NOW), vec![Step::Close { machine: "m1".into(), tab: "tab-1".into() }]);
        assert_eq!(j.phase, running("m2"));
    }

    #[test]
    fn a_finished_turn_idles_the_run_and_a_closed_tab_does_too() {
        let mut j = job(running("m1"), 1);
        j.reconcile(&[("m1".to_owned(), vec![tab("tab-1", Some(AgentStatus::Done))])], &mut 0, NOW);
        assert_eq!(j.phase, Phase::Idle { machine: "m1".into() });
        let mut j = job(running("m1"), 1);
        j.reconcile(&[("m1".to_owned(), vec![])], &mut 0, NOW);
        assert_eq!((j.phase, j.events.last().unwrap().text.as_str()), (Phase::Idle { machine: "m1".into() }, "its tab closed"));
    }

    #[test]
    fn an_idle_run_is_never_dispatched_when_its_machine_goes_away() {
        let mut j = job(Phase::Idle { machine: "m1".into() }, 1);
        assert_eq!(j.reconcile(&[], &mut 0, NOW + 10 * GRACE), vec![]);
        assert_eq!(j.phase, Phase::Idle { machine: "m1".into() });
    }

    #[test]
    fn an_idle_run_working_again_somewhere_runs_there() {
        let mut j = job(Phase::Idle { machine: "m1".into() }, 1);
        let online = [("m2".to_owned(), vec![tab("tab-2", Some(AgentStatus::Working))])];
        assert_eq!(j.reconcile(&online, &mut 0, NOW), vec![]);
        assert_eq!(j.phase, running("m2"));
    }

    #[test]
    fn a_failed_turn_is_continued_once_per_failure_until_the_attempts_run_out() {
        let mut j = job(running("m1"), 1);
        let failed = |at| TabView { updated: Some(at), ..tab("tab-1", Some(AgentStatus::Failed)) };
        assert_eq!(j.reconcile(&[("m1".to_owned(), vec![failed(NOW)])], &mut 0, NOW), vec![Step::Continue]);
        assert_eq!(j.reconcile(&[("m1".to_owned(), vec![failed(NOW)])], &mut 0, NOW + 60), vec![]);
        assert_eq!(j.reconcile(&[("m1".to_owned(), vec![failed(NOW + 90)])], &mut 0, NOW + 120), vec![Step::Continue]);
        let error = "its turn failed and the run has used its 3 attempts".to_owned();
        assert_eq!(j.reconcile(&[("m1".to_owned(), vec![failed(NOW + 150)])], &mut 0, NOW + 180), vec![Step::Report(error.clone())]);
        assert_eq!((j.attempt, j.phase), (3, Phase::Failed { error }));
    }

    #[test]
    fn a_cancelled_run_has_every_tab_closed() {
        let mut j = job(Phase::Cancelled, 1);
        let online = [("m1".to_owned(), vec![tab("tab-1", Some(AgentStatus::Working))])];
        assert_eq!(j.reconcile(&online, &mut 0, NOW), vec![Step::Close { machine: "m1".into(), tab: "tab-1".into() }]);
    }

    #[test]
    fn a_run_keeps_its_wire_shape() {
        let j = job(running("m1"), 1);
        let value = serde_json::to_value(&j).unwrap();
        assert_eq!((value["state"].as_str(), value["machine"].as_str(), value["issue"].as_str()), (Some("running"), Some("m1"), None));
        assert_eq!(value["subject"]["issue"], "i1");
        assert_eq!(serde_json::from_value::<Job>(value).unwrap(), j);
    }
}
