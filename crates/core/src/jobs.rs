//! Jobs: one record per piece of work, from where it came in (a Linear issue, a Slack thread, a
//! delegated task, the owner, a schedule) to where it ended (merged, done or rejected).
//!
//! A job moves through the stages of its line, and `decide` alone says which moves are legal.
//! On the code line, which has every working stage, they are:
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
//! A line may leave out spec, review and merging, and then refuses every move into them. Its
//! forward moves go to the next stage it has: without spec, intake → build; without review,
//! gate → approve; without merging, approve has no forward move and the job ends in done or
//! merged as above.
//!
//! Each move into build starts a round. A gate or review that sends the job back to build once
//! `round` has reached `round_limit` lands it in needs-you instead; the owner sending it on to
//! build from there extends the cap by `max_rounds` from the current round.
//!
//! The hub stores jobs and lines beside the workflow runs, through `Storage::save_jobs` and
//! `Storage::save_lines`, and never sends them to apps.

use crate::connectors::Storage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// How many rounds a job gets before it needs you, and how many more each extension adds.
pub const DEFAULT_MAX_ROUNDS: u32 = 3;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
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

/// The stages a line may list, in their working order.
const WORKING_STAGES: [Stage; 7] = [Stage::Intake, Stage::Spec, Stage::Build, Stage::Gate, Stage::Review, Stage::Approve, Stage::Merging];

/// The working stages a line may leave out; it lists every other one.
const OPTIONAL_STAGES: [Stage; 3] = [Stage::Spec, Stage::Review, Stage::Merging];

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

/// One of a project's assembly lines: the working stages its jobs pass through, in order, and
/// who works each. Every line also has needs-info, needs-you, merged, done and rejected.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub id: String,
    /// The project's path or repo, as on `Job`.
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub name: String,
    /// Whether the line is on.
    #[serde(default)]
    pub enabled: bool,
    /// The id of the workflow that starts the line.
    #[serde(default)]
    pub workflow: String,
    pub stages: Vec<Stage>,
    #[serde(default)]
    pub agents: BTreeMap<Stage, StageAgent>,
    /// The stages that need the owner's approval.
    #[serde(default)]
    pub approvals: Vec<Stage>,
    /// The round cap.
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
}

/// Who works a stage, named as `delegate_task` names a worker.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StageAgent {
    /// A `profiles::WorkerProfile` name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Line {
    /// The built-in line with every working stage. A job with no line belongs to it; it is never
    /// stored.
    pub fn code(project: &str) -> Line {
        Line {
            id: String::new(),
            project: project.to_owned(),
            name: "code".into(),
            enabled: true,
            workflow: String::new(),
            stages: WORKING_STAGES.to_vec(),
            agents: BTreeMap::new(),
            approvals: Vec::new(),
            max_rounds: DEFAULT_MAX_ROUNDS,
        }
    }

    /// Refuses stages that are not working stages, repeat, leave the working order or leave out
    /// one that is not optional. The refusal names the stage.
    pub fn check(&self) -> Result<(), String> {
        let mut last = None;
        for (i, &stage) in self.stages.iter().enumerate() {
            let Some(at) = WORKING_STAGES.iter().position(|&s| s == stage) else { return Err(format!("{stage} is not a working stage")) };
            if self.stages[..i].contains(&stage) {
                return Err(format!("{stage} is listed twice"));
            }
            if let Some(before) = last.filter(|&before| before > at) {
                return Err(format!("{stage} is listed after {}", WORKING_STAGES[before]));
            }
            last = Some(at);
        }
        match WORKING_STAGES.into_iter().find(|stage| !OPTIONAL_STAGES.contains(stage) && !self.stages.contains(stage)) {
            Some(missing) => Err(format!("{missing} is left out")),
            None => Ok(()),
        }
    }

    /// The stage after `stage` on this line; none after its last stage or for one it doesn't have.
    pub fn next(&self, stage: Stage) -> Option<Stage> {
        let at = self.stages.iter().position(|&s| s == stage)?;
        self.stages.get(at + 1).copied()
    }
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
    /// The id of its line; empty is the code line.
    #[serde(default)]
    pub line: String,
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
        Job { id, source, workflow, project, line: String::new(), branch: None, pr: None, stage: Stage::Intake, round: 0, max_rounds, round_limit: max_rounds, cost_usd: 0.0, log: Vec::new() }
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

/// Whether `job`, on `line`, may move to `asked`, and where it lands: a gate or review asking for
/// build at the round cap lands in needs-you. A forward move goes to the next stage the line has,
/// and a move into a working stage the line doesn't have is refused. The refusal names both
/// stages.
pub fn decide(job: &Job, line: &Line, asked: Stage, by: Actor) -> Result<Move, String> {
    use Stage::*;
    let from = job.stage;
    if WORKING_STAGES.contains(&asked) && !line.stages.contains(&asked) {
        return Err(format!("a job cannot move from {from} to {asked}: the {} line has no {asked}", line.name));
    }
    let legal = match (from, asked) {
        (_, Merged) if job.pr.is_none() => return Err(format!("a job without a PR cannot move from {from} to {asked}")),
        (_, Merged) => !from.is_terminal(),
        (Approve, Done) if job.pr.is_some() => return Err(format!("a job with a PR cannot move from {from} to {asked}; it moves to merged when the PR merges")),
        _ if line.next(from) == Some(asked) => true,
        (Intake, NeedsInfo | Rejected)
        | (NeedsInfo, Intake | Rejected)
        | (Build | Gate | Review | Approve | Merging, NeedsYou)
        | (Gate | Review, Build)
        | (Approve, Done)
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

/// The line `job` belongs to: the code line when it names none, else the first of `lines` with
/// its id and project, once that line passes `check`. Any other id is an error.
pub fn line_of(job: &Job, lines: &[Line]) -> Result<Line, String> {
    if job.line.is_empty() {
        return Ok(Line::code(&job.project));
    }
    let line = lines.iter().find(|line| line.id == job.line && line.project == job.project).ok_or_else(|| format!("{} has no line {}", job.project, job.line))?;
    line.check().map_err(|e| format!("line {} of {}: {e}", line.id, line.project))?;
    Ok(line.clone())
}

/// The lines `storage` keeps; none when it keeps nothing.
pub fn load_lines<S: Storage + ?Sized>(storage: &S) -> Result<Vec<Line>, String> {
    let Some(bytes) = storage.load_lines() else { return Ok(Vec::new()) };
    serde_json::from_slice(&bytes).map_err(|e| format!("could not read lines: {e}"))
}

pub fn save_lines<S: Storage + ?Sized>(storage: &S, lines: &[Line]) -> Result<(), String> {
    let bytes = serde_json::to_vec(lines).map_err(|e| format!("could not write lines: {e}"))?;
    storage.save_lines(&bytes)
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

    /// The lean line's moves legal with or without a PR, listed apart from `decide`.
    const LEAN_LEGAL: [(Stage, Stage); 16] = [
        (Intake, Build),
        (Intake, NeedsInfo),
        (Intake, Rejected),
        (NeedsInfo, Intake),
        (NeedsInfo, Rejected),
        (Build, Gate),
        (Build, NeedsYou),
        (Gate, Approve),
        (Gate, Build),
        (Gate, NeedsYou),
        (Review, Build),
        (Review, NeedsYou),
        (Approve, NeedsYou),
        (Merging, NeedsYou),
        (NeedsYou, Build),
        (NeedsYou, Rejected),
    ];

    fn job_at(stage: Stage, pr: Option<u64>) -> Job {
        Job { stage, pr, ..Job::new("j".into(), Source::Owner, "w".into(), "/p".into(), DEFAULT_MAX_ROUNDS) }
    }

    /// A line of `/p` whose id and name are `name`.
    fn line(name: &str, stages: &[Stage]) -> Line {
        Line { id: name.into(), name: name.into(), stages: stages.to_vec(), ..Line::code("/p") }
    }

    /// A line without spec, review and merging.
    fn lean() -> Line {
        line("lean", &[Intake, Build, Gate, Approve])
    }

    #[test]
    fn decide_allows_exactly_the_legal_moves_of_all_144_pairs_with_and_without_a_pr() {
        for pr in [None, Some(7)] {
            for from in ALL {
                for asked in ALL {
                    let expected = LEGAL.contains(&(from, asked))
                        || (asked == Merged && pr.is_some() && NOT_TERMINAL.contains(&from))
                        || (from == Approve && asked == Done && pr.is_none());
                    let job = job_at(from, pr);
                    let decided = decide(&job, &line_of(&job, &[]).unwrap(), asked, Actor::Owner);
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
        let code = Line::code("/p");
        for from in NOT_TERMINAL {
            assert!(decide(&job_at(from, Some(1)), &code, Merged, Actor::Hub).is_ok(), "{from} → merged with a PR");
            assert!(decide(&job_at(from, None), &code, Merged, Actor::Hub).is_err(), "{from} → merged without a PR");
        }
        for from in [Merged, Done, Rejected] {
            assert!(decide(&job_at(from, Some(1)), &code, Merged, Actor::Hub).is_err(), "nothing leaves {from}");
        }
        assert!(decide(&job_at(Approve, None), &code, Done, Actor::Owner).is_ok());
        assert!(decide(&job_at(Approve, Some(1)), &code, Done, Actor::Owner).is_err());
    }

    #[test]
    fn a_gate_or_review_asking_for_build_at_the_round_cap_lands_in_needs_you() {
        for from in [Gate, Review] {
            for (round, to) in [(2, Build), (3, NeedsYou), (4, NeedsYou)] {
                let job = Job { round, round_limit: 3, ..job_at(from, None) };
                let mv = decide(&job, &Line::code("/p"), Build, Actor::Captain).unwrap();
                assert_eq!((mv.from, mv.to, mv.asked), (from, to, Build), "{from} at round {round} of 3");
            }
        }
    }

    #[test]
    fn needs_you_to_build_extends_the_cap_by_max_rounds_from_the_current_round() {
        let mut job = Job { round: 3, round_limit: 3, max_rounds: 3, ..job_at(NeedsYou, None) };
        let mv = decide(&job, &Line::code("/p"), Build, Actor::Owner).unwrap();
        job.apply(mv, "2026-11-23T10:00:00Z".into());
        assert_eq!((job.stage, job.round, job.round_limit), (Build, 4, 6));
    }

    #[test]
    fn spec_to_build_starts_round_one_and_moves_that_do_not_build_keep_the_round() {
        let (mut job, code) = (job_at(Spec, None), Line::code("/p"));
        job.apply(decide(&job, &code, Build, Actor::Orchestrator).unwrap(), "t1".into());
        assert_eq!((job.stage, job.round, job.round_limit), (Build, 1, 3));
        job.apply(decide(&job, &code, Gate, Actor::Hub).unwrap(), "t2".into());
        assert_eq!((job.stage, job.round), (Gate, 1));
    }

    #[test]
    fn each_applied_move_logs_from_to_asked_by_and_at() {
        let mut job = Job { round: 3, round_limit: 3, ..job_at(Gate, None) };
        let (run, code) = (Actor::Run { run: "r1".into() }, Line::code("/p"));
        job.apply(decide(&job, &code, Build, run.clone()).unwrap(), "2026-11-23T10:00:00Z".into());
        job.apply(decide(&job, &code, Rejected, Actor::Owner).unwrap(), "2026-11-23T11:00:00Z".into());
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
        assert_eq!(job.line, "", "a job without a line is on the code line");
        assert_eq!(serde_json::to_value(NeedsYou).unwrap(), "needs-you");
        assert_eq!(serde_json::to_value(NeedsInfo).unwrap(), "needs-info");

        let full = Job {
            id: "j2".into(),
            source: Source::SlackThread { channel: "C1".into(), thread: "1.2".into() },
            workflow: "fix".into(),
            project: "owner/repo".into(),
            line: "oncall".into(),
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
        assert!(text.contains(r#""line":"oncall""#), "{text}");
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

    #[test]
    fn a_job_with_no_line_gets_the_code_line() {
        let job = job_at(Intake, None);
        let line = line_of(&job, &[lean(), line("", &[Intake, Build, Gate, Approve])]).unwrap();
        assert_eq!(line, Line::code(&job.project), "a stored line with the empty id is never reached");
        assert_eq!((line.id.as_str(), line.name.as_str(), line.max_rounds), ("", "code", 3));
        assert_eq!(line.stages, [Intake, Spec, Build, Gate, Review, Approve, Merging]);
    }

    #[test]
    fn a_line_without_spec_review_and_merging_allows_exactly_its_moves_of_all_144_pairs_with_and_without_a_pr() {
        let lean = lean();
        for pr in [None, Some(7)] {
            for from in ALL {
                for asked in ALL {
                    let expected = LEAN_LEGAL.contains(&(from, asked))
                        || (asked == Merged && pr.is_some() && NOT_TERMINAL.contains(&from))
                        || (from == Approve && asked == Done && pr.is_none());
                    let decided = decide(&job_at(from, pr), &lean, asked, Actor::Owner);
                    assert_eq!(decided.is_ok(), expected, "{from} → {asked} with pr {pr:?}: {decided:?}");
                    match decided {
                        Ok(mv) => assert_eq!(mv, Move { from, to: asked, asked, by: Actor::Owner }, "below the cap a move lands where asked"),
                        Err(why) => {
                            assert!(why.contains(&from.to_string()) && why.contains(&asked.to_string()), "the refusal names both stages: {why}");
                            if [Spec, Review, Merging].contains(&asked) {
                                assert!(why.contains("lean"), "a move into a stage the line lacks names the line: {why}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_line_without_only_spec_review_or_merging_skips_that_stage_and_keeps_the_other_rules() {
        let fast = line("fast", &[Intake, Build, Gate, Review, Approve, Merging]);
        let mut job = job_at(Intake, None);
        job.apply(decide(&job, &fast, Build, Actor::Captain).unwrap(), "t1".into());
        assert_eq!((job.stage, job.round), (Build, 1), "intake → build starts round 1");
        let why = decide(&job_at(Intake, None), &fast, Spec, Actor::Captain).unwrap_err();
        assert!(why.contains("spec") && why.contains("fast"), "{why}");

        let trusted = line("trusted", &[Intake, Spec, Build, Gate, Approve, Merging]);
        assert_eq!(decide(&job_at(Gate, None), &trusted, Approve, Actor::Hub).map(|mv| mv.to), Ok(Approve));
        assert!(decide(&job_at(Gate, None), &trusted, Review, Actor::Hub).is_err());
        let capped = Job { round: 3, round_limit: 3, ..job_at(Gate, None) };
        assert_eq!(decide(&capped, &trusted, Build, Actor::Hub).map(|mv| mv.to), Ok(NeedsYou), "the round cap still holds");

        let local = line("local", &[Intake, Spec, Build, Gate, Review, Approve]);
        assert!(decide(&job_at(Approve, None), &local, Merging, Actor::Owner).is_err());
        assert_eq!(decide(&job_at(Approve, None), &local, Done, Actor::Owner).map(|mv| mv.to), Ok(Done));
        assert!(decide(&job_at(Approve, Some(4)), &local, Done, Actor::Owner).is_err(), "a job with a PR ends when it merges");
        assert_eq!(decide(&job_at(Approve, Some(4)), &local, Merged, Actor::Hub).map(|mv| mv.to), Ok(Merged));
    }

    #[test]
    fn next_is_the_following_stage_the_line_has() {
        let (code, lean) = (Line::code("/p"), lean());
        let walked: Vec<Stage> = std::iter::successors(Some(Intake), |&stage| code.next(stage)).collect();
        assert_eq!(walked, [Intake, Spec, Build, Gate, Review, Approve, Merging], "the code line ends at merging");
        assert_eq!((lean.next(Intake), lean.next(Gate), lean.next(Approve), lean.next(Spec)), (Some(Build), Some(Approve), None, None));
        assert_eq!((code.next(NeedsYou), lean.next(NeedsYou)), (None, None));
    }

    #[test]
    fn check_takes_lines_that_leave_out_optional_stages_and_names_the_stage_at_fault_otherwise() {
        assert_eq!(Line::code("/p").check(), Ok(()));
        for stages in [&[Intake, Build, Gate, Approve][..], &[Intake, Build, Gate, Review, Approve, Merging], &[Intake, Spec, Build, Gate, Approve, Merging], &[Intake, Spec, Build, Gate, Review, Approve]] {
            assert_eq!(line("l", stages).check(), Ok(()), "{stages:?}");
        }
        for (stages, why) in [
            (&[Intake, Build, Gate, Approve, NeedsYou][..], "needs-you is not a working stage"),
            (&[Intake, Build, Build, Gate, Approve], "build is listed twice"),
            (&[Intake, Gate, Build, Approve], "build is listed after gate"),
            (&[Build, Gate, Approve], "intake is left out"),
            (&[Intake, Gate, Approve], "build is left out"),
            (&[Intake, Build, Approve], "gate is left out"),
            (&[Intake, Build, Gate], "approve is left out"),
        ] {
            assert_eq!(line("l", stages).check(), Err(why.to_owned()), "{stages:?}");
        }
    }

    #[test]
    fn line_of_finds_the_stored_line_of_the_jobs_project_and_never_falls_back_to_the_code_line() {
        let oncall = line("oncall", &[Intake, Build, Gate, Approve]);
        let on = |project: &str, id: &str| Job { project: project.into(), line: id.into(), ..job_at(Intake, None) };
        assert_eq!(line_of(&on("/p", "oncall"), &[lean(), oncall.clone()]), Ok(oncall.clone()));
        let why = line_of(&on("/p", "nope"), &[oncall.clone()]).unwrap_err();
        assert!(why.contains("nope"), "{why}");
        let why = line_of(&on("/q", "oncall"), &[oncall]).unwrap_err();
        assert!(why.contains("oncall"), "a line of another project is not the job's: {why}");
        let why = line_of(&on("/p", "oncall"), &[line("oncall", &[Intake, Build, Approve])]).unwrap_err();
        assert!(why.contains("gate is left out"), "a stored line that fails check carries the reason: {why}");
    }

    #[test]
    fn a_minimal_line_reads_with_defaults_and_a_full_line_round_trips() {
        let read: Line = serde_json::from_str(r#"{"id":"l","stages":["intake","build","gate","approve"]}"#).unwrap();
        let minimal = Line { id: "l".into(), project: String::new(), name: String::new(), enabled: false, workflow: String::new(), stages: vec![Intake, Build, Gate, Approve], agents: BTreeMap::new(), approvals: vec![], max_rounds: 3 };
        assert_eq!(read, minimal);

        let full = Line {
            id: "oncall".into(),
            project: "owner/repo".into(),
            name: "on-call".into(),
            enabled: true,
            workflow: "errors".into(),
            stages: vec![Intake, Build, Gate, Approve],
            agents: BTreeMap::from([(Build, StageAgent { profile: Some("fixer".into()), model: Some("opus".into()) }), (Gate, StageAgent { profile: None, model: Some("haiku".into()) })]),
            approvals: vec![Approve],
            max_rounds: 5,
        };
        let text = serde_json::to_string(&full).unwrap();
        assert!(text.contains(r#""agents":{"build":{"profile":"fixer","model":"opus"},"gate":{"model":"haiku"}}"#), "{text}");
        assert!(text.contains(r#""stages":["intake","build","gate","approve"]"#), "{text}");
        assert_eq!(serde_json::from_str::<Line>(&text).unwrap(), full);
    }

    #[test]
    fn file_storage_keeps_the_lines_in_lines_json_beside_the_store_the_runs_and_the_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let storage = FileStorage::new(dir.path().to_path_buf());
        assert_eq!(load_lines(&storage).unwrap(), vec![], "no lines.json reads as no lines");
        let jobs = vec![job_at(Build, None)];
        save(&storage, &jobs).unwrap();
        let lines = vec![lean(), Line { approvals: vec![Approve], ..line("oncall", &[Intake, Spec, Build, Gate, Approve]) }];
        save_lines(&storage, &lines).unwrap();
        assert!(dir.path().join("lines.json").exists());
        assert_eq!(load_lines(&storage).unwrap(), lines);
        assert_eq!(load(&storage).unwrap(), jobs, "saving the lines keeps the jobs");

        storage.save(br#"{"n":1}"#).unwrap();
        storage.save_runs(b"[]").unwrap();
        save(&storage, &jobs).unwrap();
        let dyn_storage: &dyn Storage = &storage;
        assert_eq!(load_lines(dyn_storage).unwrap(), lines, "saving the store, the runs and the jobs keeps the lines");

        std::fs::write(dir.path().join("lines.json"), b"{").unwrap();
        assert!(load_lines(&storage).is_err(), "lines that do not parse are an error");
    }
}
