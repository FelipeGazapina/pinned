//! The Reviewer (#341): a session that reads a worker's change against the account's and the
//! project's review rules once the worker's turn ends, and sends what it finds back to the worker.
//! Its rules, model, rounds and the project switch are the user's own: only the hub's
//! `set_reviewer` writes them, which no agent may call (`surface::ACTIONS`), and they are no field
//! of `Taste` or `TastePatch`, so the taste tools cannot name them.
//!
//! Owners: U4 (store helpers below), U8 (running a review, `machine/reviewer.rs`), U9
//! (`send_back`, `machine/reviewer.rs`).

use crate::taste::{ModelRef, Tastes};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Rounds a handoff chain sends findings back before the Captain hears, when the user set none.
pub const DEFAULT_ROUNDS: u8 = 2;
/// The most rounds the user may set.
pub const MAX_ROUNDS: u8 = 5;
/// The longest `Finding::what`, in characters.
pub const FINDING_CHARS: usize = 300;
/// The most rules the account, or one project, may have.
pub const MAX_RULES: usize = 30;
/// The longest rule text, in characters once trimmed.
pub const RULE_CHARS: usize = 500;
/// The most projects with their own review settings.
pub const MAX_PROJECTS: usize = 200;

/// The account's review settings, kept in `Tastes::reviewer`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct Reviewer {
    /// The model reviews run on. None picks the next model down the ladder from the hard-work model.
    pub model: Option<ModelRef>,
    /// Rounds of findings sent back before the Captain hears, 1 to 5.
    pub rounds: u8,
    /// The account's rules, A1, A2 and on.
    pub rules: Vec<Rule>,
    /// Each project's switch and rules, by repo as in `github.com/acme/site`.
    pub projects: BTreeMap<String, ProjectReview>,
}

impl Default for Reviewer {
    fn default() -> Reviewer {
        Reviewer { model: None, rounds: DEFAULT_ROUNDS, rules: vec![], projects: BTreeMap::new() }
    }
}

/// One project's review settings.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct ProjectReview {
    pub switch: Switch,
    /// The project's rules, P1, P2 and on.
    pub rules: Vec<Rule>,
}

/// Whether a project's workers get a review.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Switch {
    /// pinned decides by the change.
    #[default]
    Auto,
    On,
    Off,
}

/// One review rule.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Rule {
    /// A1 and on for the account's, P1 and on for a project's.
    pub id: String,
    pub text: String,
    /// The account rule a project rule stands in for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
}

/// What `HubCall::SetReviewer` changes; a field left out stays. `rules` and `switch` are the
/// project's when `project` names one, else `rules` are the account's.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct ReviewerPatch {
    /// A model sets it; null clears it back to the ladder's pick.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    #[schemars(with = "Option<ModelRef>")]
    pub model: Option<Option<ModelRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounds: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<Rule>>,
    /// The project's repo, as in `github.com/acme/site`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch: Option<Switch>,
}

/// A field that is present reads as `Some`, null included.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(de).map(Some)
}

/// One round of a task's review, which `task_status` answers in `reviews`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct ReviewRound {
    /// 1 and on.
    pub round: u8,
    #[serde(default)]
    pub findings: Vec<Finding>,
    pub state: ReviewState,
    /// The model the review ran on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// What the card says about the round, such as why the model differs from the one set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// No finding: the change passed.
    Passed,
    /// Findings with rounds left, waiting on the Captain: `task_fix` sends them back, or
    /// `task_review` accepts the work (U9).
    Found,
    /// The findings went back to the worker.
    Sent,
    /// The rounds ran out with findings left, and the Captain heard.
    Limit,
    /// The review did not run, or its answer could not judge the change; `note` says why. Never
    /// a pass.
    NotReviewed,
}

/// One thing a review found, against one rule.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Finding {
    /// The rule's id, as A1 or P2.
    pub rule: String,
    pub file: String,
    #[serde(default)]
    pub lines: Vec<u32>,
    /// What is wrong, at most 300 characters.
    pub what: String,
}

/// What the Captain's task-end news adds about task `task`'s last review round (U9): findings
/// that wait on it, or rounds that ran out with findings left. Empty otherwise.
pub fn review_news(task: &str, reviews: &[ReviewRound]) -> String {
    match reviews.last() {
        Some(last) if last.state == ReviewState::Found => format!(
            " Reviewer round {}: {}. Send them back to the worker with task_fix taskId \"{task}\", or accept the work with task_review.",
            last.round,
            crate::delegate::many_of(last.findings.len(), "finding")
        ),
        Some(last) if last.state == ReviewState::Limit => format!(" The {}.", rounds_ran_out(last.findings.len())),
        _ => String::new(),
    }
}

/// What the Captain hears when the Reviewer's last round still had `left` findings.
pub fn rounds_ran_out(left: usize) -> String {
    format!("Reviewer's rounds ran out with {} left; stop and ask the user what to do", crate::delegate::many_of(left, "finding"))
}

/// Who sends a round's findings back to the worker.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendBackBy {
    /// The Captain, with `task_fix`.
    Captain,
    /// pinned itself, in a handoff chain.
    Chain,
}

impl Tastes {
    /// `HubCall::SetReviewer`: applies `patch` to the account's review settings, or changes nothing
    /// when the result fails `Reviewer::check`.
    pub fn set_reviewer(&mut self, patch: ReviewerPatch) -> Result<(), String> {
        let mut changed = self.reviewer.clone();
        let rules = patch.rules.map(|rules| rules.into_iter().map(|rule| Rule { text: rule.text.trim().to_owned(), ..rule }).collect());
        if let Some(model) = patch.model {
            changed.model = model;
        }
        if let Some(rounds) = patch.rounds {
            changed.rounds = rounds;
        }
        match patch.project.as_deref().map(str::trim) {
            Some("") => return Err("a project's review settings need the project's repo".into()),
            Some(repo) => {
                let project = changed.projects.entry(repo.to_owned()).or_default();
                if let Some(switch) = patch.switch {
                    project.switch = switch;
                }
                if let Some(rules) = rules {
                    project.rules = rules;
                }
                if *project == ProjectReview::default() {
                    changed.projects.remove(repo);
                }
            }
            None if patch.switch.is_some() => return Err("the review switch belongs to a project: name it in project".into()),
            None => {
                if let Some(rules) = rules {
                    changed.rules = rules;
                }
            }
        }
        changed.check()?;
        self.reviewer = changed;
        Ok(())
    }
}

impl Reviewer {
    /// The rules a review in `repo`'s project reads: the account's, less those a project rule
    /// replaces, then the project's.
    pub fn resolved(&self, repo: &str) -> Vec<&Rule> {
        let project = self.projects.get(repo).map_or(&[][..], |p| &p.rules[..]);
        let replaced = |id: &str| project.iter().any(|rule| rule.replaces.as_deref() == Some(id));
        self.rules.iter().filter(|rule| !replaced(&rule.id)).chain(project).collect()
    }

    /// Whether `repo`'s workers get a review: the project's switch, or Auto when it has none.
    pub fn switch(&self, repo: &str) -> Switch {
        self.projects.get(repo).map_or(Switch::Auto, |p| p.switch)
    }

    pub fn check(&self) -> Result<(), String> {
        if !(1..=MAX_ROUNDS).contains(&self.rounds) {
            return Err(format!("review rounds go from 1 to {MAX_ROUNDS}"));
        }
        if self.model.as_ref().is_some_and(|m| m.model.trim().is_empty()) {
            return Err("the reviewer's model needs a name".into());
        }
        if self.projects.len() > MAX_PROJECTS {
            return Err(format!("review settings cover up to {MAX_PROJECTS} projects, not {}", self.projects.len()));
        }
        check_rules(&self.rules, 'A', "the account's")?;
        if let Some(rule) = self.rules.iter().find(|rule| rule.replaces.is_some()) {
            return Err(format!("rule {} is an account rule: only a project rule replaces one", rule.id));
        }
        for (repo, project) in &self.projects {
            let whose = format!("{repo}'s");
            check_rules(&project.rules, 'P', &whose)?;
            for rule in &project.rules {
                if let Some(replaces) = rule.replaces.as_deref().filter(|id| !self.rules.iter().any(|a| a.id == *id)) {
                    return Err(format!("{whose} rule {} replaces {replaces}, which is no account rule", rule.id));
                }
            }
        }
        Ok(())
    }
}

fn check_rules(rules: &[Rule], prefix: char, whose: &str) -> Result<(), String> {
    if rules.len() > MAX_RULES {
        return Err(format!("{whose} rules go up to {MAX_RULES}, not {}", rules.len()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for rule in rules {
        let numbered = rule.id.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !numbered {
            return Err(format!("{whose} rule ids go {prefix}1, {prefix}2 and on, not {:?}", rule.id));
        }
        if !seen.insert(rule.id.as_str()) {
            return Err(format!("{whose} rules name {} twice", rule.id));
        }
        let text = rule.text.trim();
        if text.is_empty() {
            return Err(format!("rule {} says nothing", rule.id));
        }
        if text.chars().count() > RULE_CHARS {
            return Err(format!("rule {} runs past {RULE_CHARS} characters", rule.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(id: &str, text: &str) -> Rule {
        Rule { id: id.into(), text: text.into(), replaces: None }
    }

    #[test]
    fn a_taste_json_from_before_the_reviewer_reads_with_its_defaults() {
        let tastes: Tastes = serde_json::from_value(json!({ "projects": {} })).unwrap();
        assert_eq!(tastes.reviewer, Reviewer::default());
        assert_eq!(tastes.reviewer.rounds, 2);
        let project: ProjectReview = serde_json::from_value(json!({})).unwrap();
        assert_eq!(project.switch, Switch::Auto);
    }

    #[test]
    fn every_review_type_round_trips() {
        let reviewer = Reviewer {
            model: Some(ModelRef { provider: None, model: "claude-sonnet-5-5".into() }),
            rounds: 3,
            rules: vec![rule("A1", "No unwrap in the hub.")],
            projects: BTreeMap::from([("github.com/acme/site".into(), ProjectReview { switch: Switch::On, rules: vec![Rule { replaces: Some("A1".into()), ..rule("P1", "No unwrap.") }] })]),
        };
        let value = serde_json::to_value(&reviewer).unwrap();
        assert_eq!(value["projects"]["github.com/acme/site"], json!({ "switch": "on", "rules": [{ "id": "P1", "text": "No unwrap.", "replaces": "A1" }] }));
        assert_eq!(serde_json::from_value::<Reviewer>(value).unwrap(), reviewer);

        let round = ReviewRound {
            round: 1,
            findings: vec![Finding { rule: "A1".into(), file: "src/hub.rs".into(), lines: vec![12, 40], what: "unwrap on a socket read".into() }],
            state: ReviewState::Sent,
            model: Some("claude-sonnet-5-5".into()),
            note: None,
        };
        let value = serde_json::to_value(&round).unwrap();
        assert_eq!(value["state"], "sent");
        assert_eq!(serde_json::from_value::<ReviewRound>(value).unwrap(), round);
        for (state, name) in [(ReviewState::Passed, "passed"), (ReviewState::Sent, "sent"), (ReviewState::Limit, "limit"), (ReviewState::NotReviewed, "not_reviewed")] {
            assert_eq!(serde_json::to_value(state).unwrap(), json!(name));
        }
        for (by, name) in [(SendBackBy::Captain, "captain"), (SendBackBy::Chain, "chain")] {
            assert_eq!(serde_json::from_value::<SendBackBy>(json!(name)).unwrap(), by);
        }

        let patch = ReviewerPatch { model: Some(None), rounds: Some(4), rules: None, project: Some("github.com/acme/site".into()), switch: Some(Switch::Off) };
        let value = serde_json::to_value(&patch).unwrap();
        assert_eq!(value, json!({ "model": null, "rounds": 4, "project": "github.com/acme/site", "switch": "off" }));
        assert_eq!(serde_json::from_value::<ReviewerPatch>(value).unwrap(), patch);
        assert_eq!(serde_json::from_value::<ReviewerPatch>(json!({})).unwrap(), ReviewerPatch::default(), "a left-out model stays");
    }

    #[test]
    fn set_reviewer_stores_what_it_is_given_and_refuses_a_bad_change_whole() {
        let mut tastes = Tastes::default();
        tastes.set_reviewer(ReviewerPatch { rounds: Some(3), rules: Some(vec![rule("A1", "Tests first.")]), ..ReviewerPatch::default() }).unwrap();
        tastes.set_reviewer(ReviewerPatch { project: Some("github.com/acme/site".into()), switch: Some(Switch::On), rules: Some(vec![rule("P1", "No new deps.")]), ..ReviewerPatch::default() }).unwrap();
        assert_eq!((tastes.reviewer.rounds, tastes.reviewer.rules.len()), (3, 1));
        assert_eq!(tastes.reviewer.projects["github.com/acme/site"].switch, Switch::On);

        let before = tastes.clone();
        assert_eq!(tastes.set_reviewer(ReviewerPatch { rounds: Some(6), ..ReviewerPatch::default() }), Err("review rounds go from 1 to 5".into()));
        assert_eq!(tastes.set_reviewer(ReviewerPatch { rounds: Some(0), ..ReviewerPatch::default() }), Err("review rounds go from 1 to 5".into()));
        assert_eq!(tastes.set_reviewer(ReviewerPatch { switch: Some(Switch::Off), ..ReviewerPatch::default() }), Err("the review switch belongs to a project: name it in project".into()));
        assert_eq!(tastes.set_reviewer(ReviewerPatch { rules: Some(vec![rule("P1", "x")]), ..ReviewerPatch::default() }), Err("the account's rule ids go A1, A2 and on, not \"P1\"".into()));
        assert_eq!(tastes.set_reviewer(ReviewerPatch { rules: Some(vec![rule("A1", "x"), rule("A1", "y")]), ..ReviewerPatch::default() }), Err("the account's rules name A1 twice".into()));
        assert_eq!(tastes, before, "a refused change changed the settings");

        tastes.set_reviewer(ReviewerPatch { project: Some("github.com/acme/site".into()), switch: Some(Switch::Auto), rules: Some(vec![]), ..ReviewerPatch::default() }).unwrap();
        assert!(tastes.reviewer.projects.is_empty(), "a project back to the defaults is dropped");
    }

    fn rules(prefix: char, n: usize) -> Vec<Rule> {
        (1..=n).map(|i| rule(&format!("{prefix}{i}"), "Tests first.")).collect()
    }

    fn ids<'a>(rules: Vec<&'a Rule>) -> Vec<&'a str> {
        rules.into_iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn a_project_rule_that_replaces_an_account_rule_drops_it_from_that_projects_list() {
        let mut tastes = Tastes::default();
        tastes.set_reviewer(ReviewerPatch { rules: Some(vec![rule("A1", "No unwrap."), rule("A2", "Tests first.")]), ..ReviewerPatch::default() }).unwrap();
        let p1 = Rule { replaces: Some("A1".into()), ..rule("P1", "No unwrap outside tests.") };
        tastes.set_reviewer(ReviewerPatch { project: Some("github.com/acme/site".into()), switch: Some(Switch::On), rules: Some(vec![p1, rule("P2", "No new deps.")]), ..ReviewerPatch::default() }).unwrap();
        tastes.set_reviewer(ReviewerPatch { project: Some("github.com/acme/docs".into()), switch: Some(Switch::Off), ..ReviewerPatch::default() }).unwrap();

        let reviewer = &tastes.reviewer;
        assert_eq!(ids(reviewer.resolved("github.com/acme/site")), ["A2", "P1", "P2"]);
        assert_eq!(ids(reviewer.resolved("github.com/acme/docs")), ["A1", "A2"]);
        assert_eq!(ids(reviewer.resolved("github.com/acme/other")), ["A1", "A2"], "a project with no settings reads the account's");
        assert_eq!(reviewer.switch("github.com/acme/site"), Switch::On);
        assert_eq!(reviewer.switch("github.com/acme/docs"), Switch::Off);
        assert_eq!(reviewer.switch("github.com/acme/other"), Switch::Auto);
        assert_eq!(reviewer.rounds, 2, "rounds stay at the default");

        let before = tastes.clone();
        let without_a1 = ReviewerPatch { rules: Some(vec![rule("A2", "Tests first.")]), ..ReviewerPatch::default() };
        assert_eq!(tastes.set_reviewer(without_a1), Err("github.com/acme/site's rule P1 replaces A1, which is no account rule".into()));
        assert_eq!(tastes, before);
    }

    #[test]
    fn each_cap_refuses_one_past_its_limit_and_stores_nothing() {
        let site = || Some("github.com/acme/site".to_owned());
        let mut tastes = Tastes::default();
        tastes.set_reviewer(ReviewerPatch { rules: Some(rules('A', 30)), ..ReviewerPatch::default() }).unwrap();
        tastes.set_reviewer(ReviewerPatch { project: site(), rules: Some(rules('P', 30)), ..ReviewerPatch::default() }).unwrap();
        let long = format!("  {}  ", "x".repeat(500));
        tastes.set_reviewer(ReviewerPatch { project: Some("github.com/acme/long".into()), rules: Some(vec![rule("P1", &long)]), ..ReviewerPatch::default() }).unwrap();
        assert_eq!(tastes.reviewer.projects["github.com/acme/long"].rules[0].text, "x".repeat(500), "the text is kept trimmed");
        for i in 3..=200 {
            tastes.set_reviewer(ReviewerPatch { project: Some(format!("github.com/acme/r{i}")), switch: Some(Switch::On), ..ReviewerPatch::default() }).unwrap();
        }
        assert_eq!(tastes.reviewer.projects.len(), 200);

        let before = tastes.clone();
        let refusals = [
            (ReviewerPatch { rules: Some(rules('A', 31)), ..ReviewerPatch::default() }, "the account's rules go up to 30, not 31"),
            (ReviewerPatch { project: site(), rules: Some(rules('P', 31)), ..ReviewerPatch::default() }, "github.com/acme/site's rules go up to 30, not 31"),
            (ReviewerPatch { rules: Some(vec![rule("A1", &"x".repeat(501))]), ..ReviewerPatch::default() }, "rule A1 runs past 500 characters"),
            (ReviewerPatch { project: site(), rules: Some(vec![rule("P1", &format!(" {} ", "y".repeat(501)))]), ..ReviewerPatch::default() }, "rule P1 runs past 500 characters"),
            (ReviewerPatch { rules: Some(vec![rule("A1", "  \n ")]), ..ReviewerPatch::default() }, "rule A1 says nothing"),
            (ReviewerPatch { project: site(), rules: Some(vec![rule("P1", "x"), rule("P1", "y")]), ..ReviewerPatch::default() }, "github.com/acme/site's rules name P1 twice"),
            (ReviewerPatch { project: site(), rules: Some(vec![Rule { replaces: Some("A31".into()), ..rule("P1", "x") }]), ..ReviewerPatch::default() }, "github.com/acme/site's rule P1 replaces A31, which is no account rule"),
            (ReviewerPatch { rules: Some(vec![Rule { replaces: Some("A2".into()), ..rule("A1", "x") }]), ..ReviewerPatch::default() }, "rule A1 is an account rule: only a project rule replaces one"),
            (ReviewerPatch { project: Some("github.com/acme/r201".into()), switch: Some(Switch::On), ..ReviewerPatch::default() }, "review settings cover up to 200 projects, not 201"),
        ];
        for (patch, refusal) in refusals {
            assert_eq!(tastes.set_reviewer(patch.clone()), Err(refusal.to_owned()), "{patch:?}");
            assert_eq!(tastes, before, "a refused change stored something: {patch:?}");
        }
    }
}
