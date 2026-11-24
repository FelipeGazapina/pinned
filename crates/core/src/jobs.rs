//! Jobs: one record per piece of work, from where it came in (a Linear issue, a Slack thread, a
//! delegated task, the owner, a schedule) to where it ended (merged, done or rejected).
//!
//! A job moves through stages, and `decide` alone says which moves are legal:
//!
//! ```text
//! intake      → spec, needs-info, rejected
//! needs-info  → intake, rejected
//! spec        → build
//! build       → gate, needs-you
//! gate        → review, build, needs-you
//! review      → approve, build, needs-you
//! approve     → merging, needs-you, done (only without a PR)
//! merging     → needs-you
//! needs-you   → build, rejected
//! any stage that is not merged, done or rejected → merged (only with a PR)
//! ```
//!
//! Each move into build starts a round. A gate or review that sends the job back to build once
//! `round` has reached `round_limit` lands it in needs-you instead; the owner sending it on to
//! build from there extends the cap by `max_rounds` from the current round.
//!
//! The hub stores jobs beside the workflow runs, through `Storage::save_jobs`, and never sends
//! them to apps.

use crate::connectors::Storage;
use serde::{Deserialize, Serialize};
use std::fmt;

/// How many rounds a job gets before it needs you, and how many more each extension adds.
pub const DEFAULT_MAX_ROUNDS: u32 = 3;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    #[default]
    Intake,
    Spec,
    Build,
    Gate,
    Review,
    Approve,
    Merging,
    NeedsInfo,
    NeedsYou,
    Merged,
    Done,
    Rejected,
}

impl Stage {
    /// Nothing leaves a terminal stage.
    pub fn is_terminal(self) -> bool {
        matches!(self, Stage::Merged | Stage::Done | Stage::Rejected)
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Stage::Intake => "intake",
            Stage::Spec => "spec",
            Stage::Build => "build",
            Stage::Gate => "gate",
            Stage::Review => "review",
            Stage::Approve => "approve",
            Stage::Merging => "merging",
            Stage::NeedsInfo => "needs-info",
            Stage::NeedsYou => "needs-you",
            Stage::Merged => "merged",
            Stage::Done => "done",
            Stage::Rejected => "rejected",
        })
    }
}

/// Where a job came from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Source {
    LinearIssue { issue: String },
    SlackThread { channel: String, thread: String },
    DelegatedTask { task: String },
    /// The owner asked for it.
    Owner,
    Schedule { schedule: String },
}

/// Who asked for a move.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Actor {
    Owner,
    Orchestrator,
    Captain,
    /// A workflow run, by its id.
    Run { run: String },
    Hub,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Job {
    pub id: String,
    pub source: Source,
    #[serde(default)]
    pub workflow: String,
    /// The project's path or repo.
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// The GitHub pull request's number.
    #[serde(default)]
    pub pr: Option<u64>,
    #[serde(default)]
    pub stage: Stage,
    /// The current round; each move into build starts the next one.
    #[serde(default)]
    pub round: u32,
    /// How many rounds each extension of the cap adds.
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
    /// The round past which a gate or review sends the job to needs-you instead of build.
    #[serde(default = "default_max_rounds")]
    pub round_limit: u32,
    #[serde(default)]
    pub cost_usd: f64,
    /// Every move so far, in order.
    #[serde(default)]
    pub log: Vec<Transition>,
}

fn default_max_rounds() -> u32 {
    DEFAULT_MAX_ROUNDS
}

/// One move in a job's log.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub from: Stage,
    pub to: Stage,
    /// The stage asked for, when the job landed elsewhere, as the round cap turns build into needs-you.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<Stage>,
    pub by: Actor,
    /// RFC 3339.
    pub at: String,
}

/// A move `decide` allowed, for `Job::apply`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    pub from: Stage,
    pub to: Stage,
    pub asked: Stage,
    pub by: Actor,
}

impl Job {
    pub fn new(id: String, source: Source, workflow: String, project: String, max_rounds: u32) -> Job {
        Job { id, source, workflow, project, branch: None, pr: None, stage: Stage::Intake, round: 0, max_rounds, round_limit: max_rounds, cost_usd: 0.0, log: Vec::new() }
    }

    /// Moves the job as `decide` allowed, at `at` (RFC 3339), and logs the move.
    pub fn apply(&mut self, mv: Move, at: String) {
        if mv.to == Stage::Build {
            if mv.from == Stage::NeedsYou {
                self.round_limit = self.round + self.max_rounds;
            }
            self.round += 1;
        }
        self.stage = mv.to;
        let asked = (mv.asked != mv.to).then_some(mv.asked);
        self.log.push(Transition { from: mv.from, to: mv.to, asked, by: mv.by, at });
    }
}

/// Whether `job` may move to `asked`, and where it lands: a gate or review asking for build at the
/// round cap lands in needs-you. The refusal names both stages.
pub fn decide(job: &Job, asked: Stage, by: Actor) -> Result<Move, String> {
    use Stage::*;
    let from = job.stage;
    let legal = match (from, asked) {
        (_, Merged) if job.pr.is_none() => return Err(format!("a job without a PR cannot move from {from} to {asked}")),
        (_, Merged) => !from.is_terminal(),
        (Approve, Done) if job.pr.is_some() => return Err(format!("a job with a PR cannot move from {from} to {asked}; it moves to merged when the PR merges")),
        (Intake, Spec | NeedsInfo | Rejected)
        | (NeedsInfo, Intake | Rejected)
        | (Spec, Build)
        | (Build, Gate | NeedsYou)
        | (Gate, Review | Build | NeedsYou)
        | (Review, Approve | Build | NeedsYou)
        | (Approve, Merging | Done | NeedsYou)
        | (Merging, NeedsYou)
        | (NeedsYou, Build | Rejected) => true,
        _ => false,
    };
    if !legal {
        return Err(format!("a job cannot move from {from} to {asked}"));
    }
    let capped = matches!(from, Gate | Review) && asked == Build && job.round >= job.round_limit;
    Ok(Move { from, to: if capped { NeedsYou } else { asked }, asked, by })
}

/// The jobs `storage` keeps; none when it keeps nothing.
pub fn load<S: Storage + ?Sized>(storage: &S) -> Result<Vec<Job>, String> {
    let Some(bytes) = storage.load_jobs() else { return Ok(Vec::new()) };
    serde_json::from_slice(&bytes).map_err(|e| format!("could not read jobs: {e}"))
}

pub fn save<S: Storage + ?Sized>(storage: &S, jobs: &[Job]) -> Result<(), String> {
    let bytes = serde_json::to_vec(jobs).map_err(|e| format!("could not write jobs: {e}"))?;
    storage.save_jobs(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::FileStorage;
    use Stage::*;

    const ALL: [Stage; 12] = [Intake, Spec, Build, Gate, Review, Approve, Merging, NeedsInfo, NeedsYou, Merged, Done, Rejected];
    const NOT_TERMINAL: [Stage; 9] = [Intake, Spec, Build, Gate, Review, Approve, Merging, NeedsInfo, NeedsYou];

    /// The moves legal with or without a PR, listed apart from `decide`'s own table.
    const LEGAL: [(Stage, Stage); 19] = [
        (Intake, Spec),
        (Intake, NeedsInfo),
        (Intake, Rejected),
        (NeedsInfo, Intake),
        (NeedsInfo, Rejected),
        (Spec, Build),
        (Build, Gate),
        (Build, NeedsYou),
        (Gate, Review),
        (Gate, Build),
        (Gate, NeedsYou),
        (Review, Approve),
        (Review, Build),
        (Review, NeedsYou),
        (Approve, Merging),
        (Approve, NeedsYou),
        (Merging, NeedsYou),
        (NeedsYou, Build),
        (NeedsYou, Rejected),
    ];

    fn job_at(stage: Stage, pr: Option<u64>) -> Job {
        Job { stage, pr, ..Job::new("j".into(), Source::Owner, "w".into(), "/p".into(), DEFAULT_MAX_ROUNDS) }
    }

    #[test]
    fn decide_allows_exactly_the_legal_moves_of_all_144_pairs_with_and_without_a_pr() {
        for pr in [None, Some(7)] {
            for from in ALL {
                for asked in ALL {
                    let expected = LEGAL.contains(&(from, asked))
                        || (asked == Merged && pr.is_some() && NOT_TERMINAL.contains(&from))
                        || (from == Approve && asked == Done && pr.is_none());
                    let decided = decide(&job_at(from, pr), asked, Actor::Owner);
                    assert_eq!(decided.is_ok(), expected, "{from} → {asked} with pr {pr:?}: {decided:?}");
                    match decided {
                        Ok(mv) => assert_eq!(mv, Move { from, to: asked, asked, by: Actor::Owner }, "below the cap a move lands where asked"),
                        Err(why) => assert!(why.contains(&from.to_string()) && why.contains(&asked.to_string()), "the refusal names both stages: {why}"),
                    }
                }
            }
        }
    }

    #[test]
    fn merged_needs_a_pr_and_done_needs_none() {
        for from in NOT_TERMINAL {
            assert!(decide(&job_at(from, Some(1)), Merged, Actor::Hub).is_ok(), "{from} → merged with a PR");
            assert!(decide(&job_at(from, None), Merged, Actor::Hub).is_err(), "{from} → merged without a PR");
        }
        for from in [Merged, Done, Rejected] {
            assert!(decide(&job_at(from, Some(1)), Merged, Actor::Hub).is_err(), "nothing leaves {from}");
        }
        assert!(decide(&job_at(Approve, None), Done, Actor::Owner).is_ok());
        assert!(decide(&job_at(Approve, Some(1)), Done, Actor::Owner).is_err());
    }

    #[test]
    fn a_gate_or_review_asking_for_build_at_the_round_cap_lands_in_needs_you() {
        for from in [Gate, Review] {
            for (round, to) in [(2, Build), (3, NeedsYou), (4, NeedsYou)] {
                let job = Job { round, round_limit: 3, ..job_at(from, None) };
                let mv = decide(&job, Build, Actor::Captain).unwrap();
                assert_eq!((mv.from, mv.to, mv.asked), (from, to, Build), "{from} at round {round} of 3");
            }
        }
    }

    #[test]
    fn needs_you_to_build_extends_the_cap_by_max_rounds_from_the_current_round() {
        let mut job = Job { round: 3, round_limit: 3, max_rounds: 3, ..job_at(NeedsYou, None) };
        let mv = decide(&job, Build, Actor::Owner).unwrap();
        job.apply(mv, "2026-11-23T10:00:00Z".into());
        assert_eq!((job.stage, job.round, job.round_limit), (Build, 4, 6));
    }

    #[test]
    fn spec_to_build_starts_round_one_and_moves_that_do_not_build_keep_the_round() {
        let mut job = job_at(Spec, None);
        job.apply(decide(&job, Build, Actor::Orchestrator).unwrap(), "t1".into());
        assert_eq!((job.stage, job.round, job.round_limit), (Build, 1, 3));
        job.apply(decide(&job, Gate, Actor::Hub).unwrap(), "t2".into());
        assert_eq!((job.stage, job.round), (Gate, 1));
    }

    #[test]
    fn each_applied_move_logs_from_to_asked_by_and_at() {
        let mut job = Job { round: 3, round_limit: 3, ..job_at(Gate, None) };
        let run = Actor::Run { run: "r1".into() };
        job.apply(decide(&job, Build, run.clone()).unwrap(), "2026-11-23T10:00:00Z".into());
        job.apply(decide(&job, Rejected, Actor::Owner).unwrap(), "2026-11-23T11:00:00Z".into());
        assert_eq!(job.stage, Rejected);
        assert_eq!(job.round, 3, "a capped move does not start a round");
        assert_eq!(
            job.log,
            vec![
                Transition { from: Gate, to: NeedsYou, asked: Some(Build), by: run, at: "2026-11-23T10:00:00Z".into() },
                Transition { from: NeedsYou, to: Rejected, asked: None, by: Actor::Owner, at: "2026-11-23T11:00:00Z".into() },
            ]
        );
    }

    #[test]
    fn a_minimal_record_reads_with_defaults_and_a_full_job_round_trips() {
        let job: Job = serde_json::from_str(r#"{"id":"j","source":{"kind":"owner"}}"#).unwrap();
        assert_eq!(job, Job::new("j".into(), Source::Owner, String::new(), String::new(), DEFAULT_MAX_ROUNDS));
        assert_eq!((job.stage, job.round, job.max_rounds, job.round_limit), (Intake, 0, 3, 3));
        assert_eq!(serde_json::to_value(NeedsYou).unwrap(), "needs-you");
        assert_eq!(serde_json::to_value(NeedsInfo).unwrap(), "needs-info");

        let full = Job {
            id: "j2".into(),
            source: Source::SlackThread { channel: "C1".into(), thread: "1.2".into() },
            workflow: "fix".into(),
            project: "owner/repo".into(),
            branch: Some("factory/j2".into()),
            pr: Some(12),
            stage: Review,
            round: 2,
            max_rounds: 4,
            round_limit: 4,
            cost_usd: 1.25,
            log: vec![Transition { from: Gate, to: NeedsYou, asked: Some(Build), by: Actor::Run { run: "r".into() }, at: "2026-11-23T10:00:00Z".into() }],
        };
        let text = serde_json::to_string(&full).unwrap();
        assert!(text.contains(r#""source":{"kind":"slack-thread","channel":"C1","thread":"1.2"}"#), "{text}");
        assert!(text.contains(r#""by":{"kind":"run","run":"r"}"#), "{text}");
        assert_eq!(serde_json::from_str::<Job>(&text).unwrap(), full);
        let unasked = Transition { from: Spec, to: Build, asked: None, by: Actor::Hub, at: "t".into() };
        assert!(!serde_json::to_string(&unasked).unwrap().contains("asked"), "asked is left out when the job landed where asked");
    }

    #[test]
    fn file_storage_keeps_the_jobs_in_jobs_json_beside_the_store_and_the_runs() {
        let dir = tempfile::tempdir().unwrap();
        let storage = FileStorage::new(dir.path().to_path_buf());
        assert_eq!(load(&storage).unwrap(), vec![], "no jobs.json reads as no jobs");
        let jobs = vec![job_at(Build, Some(3)), Job::new("k".into(), Source::LinearIssue { issue: "ENG-1".into() }, "w".into(), "/p".into(), 5)];
        save(&storage, &jobs).unwrap();
        assert!(dir.path().join("jobs.json").exists());
        assert_eq!(load(&storage).unwrap(), jobs);

        storage.save(br#"{"n":1}"#).unwrap();
        storage.save_runs(b"[]").unwrap();
        let dyn_storage: &dyn Storage = &storage;
        assert_eq!(load(dyn_storage).unwrap(), jobs, "saving the store and the runs keeps the jobs");

        std::fs::write(dir.path().join("jobs.json"), b"{").unwrap();
        assert!(load(&storage).is_err(), "jobs that do not parse are an error");
    }
}
