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
    /// The findings went back to the worker.
    Sent,
    /// The rounds ran out with findings left, and the Captain heard.
    Limit,
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
                if let Some(rules) = patch.rules {
                    project.rules = rules;
                }
                if *project == ProjectReview::default() {
                    changed.projects.remove(repo);
                }
            }
            None if patch.switch.is_some() => return Err("the review switch belongs to a project: name it in project".into()),
            None => {
                if let Some(rules) = patch.rules {
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
    pub fn check(&self) -> Result<(), String> {
        if !(1..=MAX_ROUNDS).contains(&self.rounds) {
            return Err(format!("review rounds go from 1 to {MAX_ROUNDS}"));
        }
        if self.model.as_ref().is_some_and(|m| m.model.trim().is_empty()) {
            return Err("the reviewer's model needs a name".into());
        }
        check_rules(&self.rules, 'A', "the account's")?;
        self.projects.iter().try_for_each(|(repo, project)| check_rules(&project.rules, 'P', &format!("{repo}'s")))
    }
}

fn check_rules(rules: &[Rule], prefix: char, whose: &str) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for rule in rules {
        let numbered = rule.id.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !numbered {
            return Err(format!("{whose} rule ids go {prefix}1, {prefix}2 and on, not {:?}", rule.id));
        }
        if !seen.insert(rule.id.as_str()) {
            return Err(format!("{whose} rules name {} twice", rule.id));
        }
        if rule.text.trim().is_empty() {
            return Err(format!("rule {} says nothing", rule.id));
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
        for (state, name) in [(ReviewState::Passed, "passed"), (ReviewState::Sent, "sent"), (ReviewState::Limit, "limit")] {
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
}
