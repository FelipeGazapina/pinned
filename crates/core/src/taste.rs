//! How the user wants the agents to work: which model a worker starts on and climbs to, how many
//! run at once, and what they decide alone. The global orchestrator and the Capitães each have a
//! `Taste`; a project changes some of the Capitães' knobs with a `TastePatch`. The hub keeps an
//! account's `Tastes`; a machine keeps the last ones it read in `taste.json`, and a signed-out app
//! keeps its own there.
//!
//! The delegation limits (`parallel`, `tier`, `default_effort`, `max_effort`, `budget_cents`,
//! `time_limit_min`, `box_budget_cents`) are knobs like the others, so a project overrides each on
//! its own, but pinned enforces them on `delegate_task` and on boxes instead of only telling the
//! session. `Limits` is that group, resolved.

use crate::connectors::BoxSize;
use crate::profiles::{ProfileAuthor, WorkerProfile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

const FILE: &str = "taste.json";
/// Where a machine kept its cost style and its worker cap before tastes.
const OLD_FILE: &str = "orchestration.json";
/// The old per-machine worker cap's default, now the most any scope may run at once.
const MAX_PARALLEL: u32 = 100;
/// A week. Longer than any worker should run unattended.
const MAX_TIME_LIMIT_MIN: u32 = 7 * 24 * 60;
/// Eight hours, the old default of `delegate_task`'s `limitMs`.
const DEFAULT_TIME_LIMIT_MIN: u32 = 8 * 60;
/// Far above any change a person reviews.
const MAX_ESCALATE_OVER_LINES: u32 = 100_000;
const OPUS: &str = "claude-opus-5-5";

/// A model as `orchestrator_capabilities` names it, and the provider it runs through when that matters.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct ModelRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub model: String,
}

impl ModelRef {
    fn named(model: &str) -> ModelRef {
        ModelRef { provider: None, model: model.to_owned() }
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.provider {
            Some(provider) => write!(f, "{} through {provider}", self.model),
            None => f.write_str(&self.model),
        }
    }
}

/// Refuses a model name that could reach a CLI as anything but a model: an empty one, one that
/// starts with `-`, or one with a character outside letters, digits and `-.[]_`.
pub fn check_model_name(model: &str) -> Result<(), String> {
    if model.is_empty() || model.starts_with('-') || !model.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.[]_".contains(&b)) {
        return Err(format!("{model:?} is not a model name"));
    }
    Ok(())
}

/// How a worker climbs the model ladder.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// The cheapest rung with a detailed brief, re-briefed narrower before it moves up.
    CheapWithGuidance,
    /// The cheapest rung, one rung up after two failures.
    EscalateOnFailure,
    /// The hard model from the start.
    StrongestFirst,
}

/// The model cost tier `orchestrator_capabilities` lists for each model.
pub use crate::delegate::Cost as Tier;

/// How hard a model thinks, from least to most, as `orchestrator_capabilities` names the levels.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub const ALL: [Effort; 5] = [Effort::Low, Effort::Medium, Effort::High, Effort::Xhigh, Effort::Max];

    pub fn name(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }

    /// None for a level outside the five, such as Cursor's `fast`.
    pub fn parse(name: &str) -> Option<Effort> {
        Effort::ALL.into_iter().find(|e| e.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// What the agents decide without the user.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    AskFirst,
    DecideAndTell,
    ReportAtEnd,
}

/// A named setting of the knobs other than skills and context. Derived, never stored: a taste whose
/// knobs match no preset is `Custom`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Economical,
    Balanced,
    Aggressive,
    Custom,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct SkillRef {
    pub name: String,
    pub enabled: bool,
}

/// Every knob of one scope, fully resolved.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct Taste {
    pub strategy: Strategy,
    /// Cheapest first, at least one.
    pub ladder: Vec<ModelRef>,
    /// Where hard work goes. None is the top rung of the ladder.
    pub hard_model: Option<ModelRef>,
    /// Workers at once in a project, 1 to 100. `delegate_task` refuses one more.
    pub parallel: u32,
    pub autonomy: Autonomy,
    pub review_diffs: bool,
    pub second_opinion: bool,
    pub skills: Vec<SkillRef>,
    pub context: Vec<String>,
    /// The costliest model tier a worker may run on.
    pub tier: Tier,
    /// The model a new session starts on when its start names none, on a Claude Code or OpenCode
    /// Go account that lists it. None leaves it to the CLI. Always written, null included, so a
    /// page tells a hub that has it from one that predates it.
    pub default_model: Option<ModelRef>,
    /// The effort a worker gets when the call names none, and a new session when its start names
    /// none. None leaves it to the model.
    pub default_effort: Option<Effort>,
    /// The most effort a call may ask for.
    pub max_effort: Effort,
    /// What one worker may spend, in US cents, as its transcript prices it. 0 sets no cap.
    pub budget_cents: u32,
    /// Minutes a worker runs before pinned stops it: the default and the ceiling of `limitMs`.
    pub time_limit_min: u32,
    /// A box's budget in US cents: the default and the ceiling of its `budget_usd`. 0 sets neither.
    pub box_budget_cents: u32,
    /// The size of a box opened with none named: the Captains' is the account's default, a
    /// project's overrides it. None is the built-in default, small. The orchestrator's is never
    /// set, since every box belongs to a project. Always written, null included, so a page tells a
    /// hub that has it from one that predates it.
    pub box_size: Option<BoxSize>,
    /// Escalate to the user a change that breaks an older version.
    pub escalate_breaking_change: bool,
    /// Escalate to the user a run whose process did not work as designed.
    pub escalate_process_failure: bool,
    /// Escalate to the user a change past this many changed lines, not counting tests, docs and
    /// generated files. 1 to 100000.
    pub escalate_over_lines: u32,
    /// Areas whose changes always go to the user, as free text.
    pub escalate_areas: Vec<String>,
    /// The Captain starts without the file-editing tools and hands every change to a worker. On by
    /// default. Only the Captains' and a project's scope use it.
    pub manage_only: bool,
    /// Whether the scope's own session loads Claude Code's auto-memory (`MEMORY.md`) and may
    /// write to it. Off sets `CLAUDE_CODE_DISABLE_AUTO_MEMORY` on its Claude Code process. The
    /// orchestrator's scope holds the Orchestrator's, the Captains' and a project's the Captain's.
    /// pinned's own project memory is a different thing and stays. Only the user sets it.
    #[serde(default = "claude_memory_on")]
    pub claude_memory: bool,
    /// The same for the project's workers, resolved from its project scope like its other worker
    /// limits. Only the Captains' and a project's scope use it. Only the user sets it.
    #[serde(default = "claude_memory_on")]
    pub worker_claude_memory: bool,
}

fn claude_memory_on() -> bool {
    true
}

/// A scope's delegation limits, resolved: what `delegate_task` and the boxes enforce.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Limits {
    pub parallel: u32,
    pub tier: Tier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<Effort>,
    pub max_effort: Effort,
    pub budget_cents: u32,
    pub time_limit_min: u32,
    pub box_budget_cents: u32,
}

/// A scope's own changes; a field left out inherits. A patch can set `hard_model` but not clear it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct TastePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<Strategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladder: Option<Vec<ModelRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_model: Option<ModelRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<Autonomy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_diffs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second_opinion: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<SkillRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<Tier>,
    /// A project's default model over the Captains'. A patch can set it but not clear it;
    /// resetting the field clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<ModelRef>,
    /// The effort a worker whose call names none gets, and a new session whose start names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<Effort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_effort: Option<Effort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_cents: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_limit_min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub box_budget_cents: Option<u32>,
    /// A project's box size over the account's. A patch can set it but not clear it; resetting
    /// the field clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub box_size: Option<BoxSize>,
    /// The account's placement policy. Only the Captains' and the orchestrator's scope take it:
    /// it belongs to the account, and a project's own comes later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<crate::placement::Placement>,
    /// The account's Boxes at once. Only the Captains' and the orchestrator's scope take it, and
    /// only the user's own app sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boxes_at_once: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalate_breaking_change: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalate_process_failure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalate_over_lines: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalate_areas: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manage_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_memory: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_claude_memory: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TasteScope {
    /// The global orchestrator.
    Orchestrator,
    /// Every project's Capitão.
    Captains,
    /// One project's Capitão, by its repo as in `github.com/acme/site`.
    Project { repo: String },
}

/// One knob of a `Taste`, named as its JSON field.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TasteField {
    Strategy,
    Ladder,
    HardModel,
    Parallel,
    Autonomy,
    ReviewDiffs,
    SecondOpinion,
    Skills,
    Context,
    Tier,
    DefaultModel,
    DefaultEffort,
    MaxEffort,
    BudgetCents,
    TimeLimitMin,
    BoxBudgetCents,
    Placement,
    BoxSize,
    BoxesAtOnce,
    EscalateBreakingChange,
    EscalateProcessFailure,
    EscalateOverLines,
    EscalateAreas,
    ManageOnly,
    ClaudeMemory,
    WorkerClaudeMemory,
}

impl TasteField {
    /// The JSON name of a setting only the user changes on the Way of working page: a review setting, Boxes at
    /// once, an escalation setting, Manage only or a Claude Code memory switch. None for the rest.
    pub fn review_setting(self) -> Option<&'static str> {
        match self {
            TasteField::ReviewDiffs => Some("review_diffs"),
            TasteField::SecondOpinion => Some("second_opinion"),
            TasteField::BoxesAtOnce => Some("boxes_at_once"),
            TasteField::EscalateBreakingChange => Some("escalate_breaking_change"),
            TasteField::EscalateProcessFailure => Some("escalate_process_failure"),
            TasteField::EscalateOverLines => Some("escalate_over_lines"),
            TasteField::EscalateAreas => Some("escalate_areas"),
            TasteField::ManageOnly => Some("manage_only"),
            TasteField::ClaudeMemory => Some("claude_memory"),
            TasteField::WorkerClaudeMemory => Some("worker_claude_memory"),
            _ => None,
        }
    }
}

/// The refusal of an agent's change to review or escalation settings or Manage only, which only the user makes on the Way of working page.
/// Refused whole, so the other fields of the same change wait too.
pub fn refuse_review_settings(named: impl IntoIterator<Item = &'static str>) -> Result<(), String> {
    let named: Vec<&str> = named.into_iter().collect();
    if named.is_empty() {
        return Ok(());
    }
    let verb = if named.len() == 1 { "is" } else { "are" };
    Err(format!("{} {verb} changed only by the user, on the Way of working page; nothing changed", named.join(" and ")))
}

/// The settings of `Tastes` that are no knob of a `Taste`, so no `TastePatch` carries them: the
/// user's own, written only by their own hub calls.
pub const USER_ONLY: [&str; 2] = ["reviewer", "worker_profiles"];

/// A `TastePatch` as `taste_set` and `set_taste` take it: one that names a `USER_ONLY` setting is
/// refused whole instead of read without it.
pub fn patch_without_user_only<'de, D: serde::Deserializer<'de>>(de: D) -> Result<TastePatch, D::Error> {
    use serde::de::Error;
    let value = Value::deserialize(de)?;
    let named: Vec<&str> = USER_ONLY.into_iter().filter(|field| value.get(field).is_some()).collect();
    if !named.is_empty() {
        let verb = if named.len() == 1 { "is" } else { "are" };
        return Err(D::Error::custom(format!("{} {verb} the user's own, set only on the Way of working page; nothing changed", named.join(" and "))));
    }
    serde_json::from_value(value).map_err(D::Error::custom)
}

/// An account's tastes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct Tastes {
    pub orchestrator: Taste,
    pub captains: Taste,
    /// Each project's changes over `captains`, by repo.
    pub projects: BTreeMap<String, TastePatch>,
    /// Where the account's heavy delegated work runs.
    pub placement: crate::placement::Placement,
    /// The Reviewer's rules, model, rounds and project switches: the user's own, written only by
    /// `HubCall::SetReviewer`. No `TastePatch` field, so the taste tools cannot name it.
    pub reviewer: crate::reviewer::Reviewer,
    /// The account's and each project's worker profiles: the user's own, written only by
    /// `HubCall::SetWorkerProfiles`.
    pub worker_profiles: crate::profiles::WorkerProfiles,
    /// Boxes at once: how many boxes the account's agents (the orchestrator, the Captains, their
    /// workers and the workflows) may have open at once. A box the user opens never counts. Only the
    /// user's own app sets it. Read through `boxes_limit`, which holds it to `box_cap`.
    pub boxes_at_once: u32,
    /// The hub's fixed cap on the account's open boxes, the most Boxes at once can be. The hub
    /// writes it into every answer; tastes from before it read the cap of then, 8.
    pub box_cap: u32,
}

/// Boxes at once of an account that never set it, and the hub's cap before the hub sent one.
pub const DEFAULT_BOXES_AT_ONCE: u32 = 8;

impl Default for Tastes {
    fn default() -> Tastes {
        Tastes {
            orchestrator: Taste::default(),
            captains: Taste::default(),
            projects: BTreeMap::new(),
            placement: crate::placement::Placement::default(),
            reviewer: crate::reviewer::Reviewer::default(),
            worker_profiles: crate::profiles::WorkerProfiles::default(),
            boxes_at_once: DEFAULT_BOXES_AT_ONCE,
            box_cap: DEFAULT_BOXES_AT_ONCE,
        }
    }
}

impl Tastes {
    /// The boxes the account's agents may have open at once: Boxes at once, at most the hub's cap,
    /// so a larger value an older app or the API stored reads as the cap.
    pub fn boxes_limit(&self) -> u32 {
        self.boxes_at_once.min(self.box_cap)
    }

    /// What `orchestrator_capabilities` tells an agent about boxes.
    pub fn boxes_note(&self) -> String {
        match self.boxes_limit() {
            0 => BOXES_AT_ONCE_ZERO_NOTE.to_owned(),
            1 => "Boxes at once: 1. The account's agents may have 1 Upstash Box open at once; one more is refused.".to_owned(),
            n => format!("Boxes at once: {n}. The account's agents may have {n} Upstash Boxes open at once; one more is refused."),
        }
    }

    /// The line the orchestrator's and the Captains' prompts add after their brief: none while a box
    /// is allowed, and at 0 that no box opens, so they plan on the connected machines.
    pub fn boxes_brief(&self) -> Option<String> {
        (self.boxes_limit() == 0).then(|| BOXES_AT_ONCE_ZERO_NOTE.to_owned())
    }
}

const BOXES_AT_ONCE_ZERO_NOTE: &str = "Boxes at once is 0: agents cannot start an Upstash Box on this account. Never pass target.box to delegate_task or box to send_to_captain, and never call open_box; run the work on the connected machines that orchestrator_capabilities lists under machines.";

impl Profile {
    const PRESETS: [Profile; 3] = [Profile::Economical, Profile::Balanced, Profile::Aggressive];

    /// The preset's knobs, with no skills or context. None for `Custom`.
    pub fn preset(self) -> Option<Taste> {
        match self {
            Profile::Economical => Some(Taste {
                strategy: Strategy::CheapWithGuidance,
                ladder: ladder(&["deepseek-v4.1-flash", "glm-5.3", OPUS]),
                hard_model: Some(ModelRef::named(OPUS)),
                parallel: 2,
                autonomy: Autonomy::AskFirst,
                review_diffs: true,
                second_opinion: false,
                ..balanced()
            }),
            Profile::Balanced => Some(balanced()),
            Profile::Aggressive => Some(Taste {
                strategy: Strategy::StrongestFirst,
                ladder: ladder(&[OPUS]),
                hard_model: Some(ModelRef::named(OPUS)),
                parallel: 8,
                autonomy: Autonomy::ReportAtEnd,
                review_diffs: false,
                second_opinion: true,
                ..balanced()
            }),
            Profile::Custom => None,
        }
    }
}

fn ladder(models: &[&str]) -> Vec<ModelRef> {
    models.iter().map(|m| ModelRef::named(m)).collect()
}

fn balanced() -> Taste {
    Taste {
        strategy: Strategy::EscalateOnFailure,
        ladder: ladder(&["glm-5.3", "claude-sonnet-5-5", OPUS]),
        hard_model: Some(ModelRef::named(OPUS)),
        parallel: 5,
        autonomy: Autonomy::DecideAndTell,
        review_diffs: true,
        second_opinion: false,
        skills: vec![],
        context: vec![],
        tier: Tier::High,
        default_model: None,
        default_effort: None,
        max_effort: Effort::Max,
        budget_cents: 0,
        time_limit_min: DEFAULT_TIME_LIMIT_MIN,
        box_budget_cents: 0,
        box_size: None,
        escalate_breaking_change: true,
        escalate_process_failure: true,
        escalate_over_lines: 500,
        escalate_areas: ["Release and version bumps", "CI and deploy config", "Sign-in, tokens and secrets"].map(str::to_owned).to_vec(),
        manage_only: true,
        claude_memory: true,
        worker_claude_memory: true,
    }
}

impl Default for Taste {
    fn default() -> Taste {
        balanced()
    }
}

/// The workers at once each preset had before the defaults rose. Economical kept its 2, so it is
/// not here. A taste stored with exactly one of these presets' knobs was that preset rather than a
/// choice, so it reads as the preset again instead of turning Custom.
const OLD_PRESET_PARALLEL: [(Profile, u32); 2] = [(Profile::Balanced, 3), (Profile::Aggressive, 6)];

impl Taste {
    /// The preset whose knobs these are. Skills, context and the limits other than `parallel`
    /// belong to no preset.
    pub fn profile(&self) -> Profile {
        let knobs = Taste { parallel: self.parallel, ..self.style() };
        Profile::PRESETS.into_iter().find(|p| p.preset().as_ref() == Some(&knobs)).unwrap_or(Profile::Custom)
    }

    /// Sets every knob a preset has to the preset's. `Custom` changes nothing.
    pub fn apply(&mut self, profile: Profile) {
        if let Some(preset) = profile.preset() {
            *self = Taste { strategy: preset.strategy, ladder: preset.ladder, hard_model: preset.hard_model, parallel: preset.parallel, autonomy: preset.autonomy, review_diffs: preset.review_diffs, second_opinion: preset.second_opinion, ..self.clone() };
        }
    }

    /// This taste's style knobs over the default's everything else.
    fn style(&self) -> Taste {
        Taste { strategy: self.strategy, ladder: self.ladder.clone(), hard_model: self.hard_model.clone(), autonomy: self.autonomy, review_diffs: self.review_diffs, second_opinion: self.second_opinion, ..Taste::default() }
    }

    /// Takes the preset's new workers at once when this taste was stored as that preset on the
    /// old defaults. Skills, context and the limits other than `parallel` belong to no preset, so
    /// they never count, and a knob the account really chose leaves the taste Custom.
    fn raise_parallel(&mut self) {
        for (profile, old) in OLD_PRESET_PARALLEL {
            let preset = profile.preset().unwrap();
            let stored = Taste { parallel: self.parallel, ..self.style() };
            let was_the_default = Taste { parallel: old, ..preset };
            if stored == was_the_default {
                self.parallel = preset.parallel;
            }
        }
    }

    pub fn limits(&self) -> Limits {
        Limits {
            parallel: self.parallel,
            tier: self.tier,
            default_effort: self.default_effort,
            max_effort: self.max_effort,
            budget_cents: self.budget_cents,
            time_limit_min: self.time_limit_min,
            box_budget_cents: self.box_budget_cents,
        }
    }

    pub fn check(&self) -> Result<(), String> {
        if self.ladder.is_empty() {
            return Err("the model ladder needs at least one model".into());
        }
        if self.ladder.iter().chain(&self.hard_model).any(|m| m.model.trim().is_empty()) {
            return Err("every model needs a name".into());
        }
        if let Some(default) = &self.default_model {
            check_model_name(&default.model)?;
        }
        if !(1..=MAX_PARALLEL).contains(&self.parallel) {
            return Err(format!("parallel workers go from 1 to {MAX_PARALLEL}"));
        }
        if !(1..=MAX_TIME_LIMIT_MIN).contains(&self.time_limit_min) {
            return Err(format!("the time limit goes from 1 to {MAX_TIME_LIMIT_MIN} minutes"));
        }
        if !(1..=MAX_ESCALATE_OVER_LINES).contains(&self.escalate_over_lines) {
            return Err(format!("the line limit goes from 1 to {MAX_ESCALATE_OVER_LINES} changed lines"));
        }
        if self.default_effort.is_some_and(|effort| effort > self.max_effort) {
            return Err("the default effort is above the most effort allowed".into());
        }
        Ok(())
    }

    /// What a session of this scope is told, one line per knob. The settings page previews this text.
    pub fn brief(&self) -> String {
        let mut lines = vec![match self.parallel {
            1 => "Run one worker at a time.".to_owned(),
            n => format!("Run up to {n} workers at once."),
        }];
        if let (Some((first, rest)), Some(top)) = (self.ladder.split_first(), self.hard_model.as_ref().or(self.ladder.last())) {
            lines.push(match self.strategy {
                Strategy::CheapWithGuidance => {
                    let then = rest.first().map_or("when it fails".to_owned(), |next| format!("before moving it up to {next}"));
                    format!("Start each worker on {first} with a detailed brief that names the files, the contracts and a command that proves the result, and re-brief it narrower {then}.")
                }
                Strategy::EscalateOnFailure if rest.is_empty() => format!("Start each worker on {first}, and when it fails twice ask the user."),
                Strategy::EscalateOnFailure => {
                    let rungs: Vec<String> = rest.iter().map(ModelRef::to_string).collect();
                    format!("Start each worker on {first}, and when it fails twice move it one rung up the ladder: {}.", rungs.join(", then "))
                }
                Strategy::StrongestFirst => format!("Start each worker on {top} directly, without trying a cheaper model first."),
            });
            lines.push(format!("Send hard work, such as architecture calls or a bug whose cause is unknown, straight to {top}."));
        }
        lines.push(
            match self.autonomy {
                Autonomy::AskFirst => "Ask the user before you open a pull request or delete data.",
                Autonomy::DecideAndTell => "Decide on your own and tell the user what you decided.",
                Autonomy::ReportAtEnd => "Tell the user only at the end, with the result.",
            }
            .to_owned(),
        );
        if self.review_diffs {
            lines.push("Review each worker's diff before you accept its work.".to_owned());
        }
        if self.second_opinion {
            lines.push("Have another model review a risky change before it goes ahead.".to_owned());
        }
        lines.extend(self.skills.iter().filter(|s| s.enabled).map(|s| format!("Follow the {} skill.", s.name)));
        lines.extend(self.context.iter().map(|note| note.trim()).filter(|note| !note.is_empty()).map(str::to_owned));
        lines.join("\n")
    }

    fn reset(&mut self, field: TasteField) {
        let default = Taste::default();
        match field {
            TasteField::Strategy => self.strategy = default.strategy,
            TasteField::Ladder => self.ladder = default.ladder,
            TasteField::HardModel => self.hard_model = default.hard_model,
            TasteField::Parallel => self.parallel = default.parallel,
            TasteField::Autonomy => self.autonomy = default.autonomy,
            TasteField::ReviewDiffs => self.review_diffs = default.review_diffs,
            TasteField::SecondOpinion => self.second_opinion = default.second_opinion,
            TasteField::Skills => self.skills = default.skills,
            TasteField::Context => self.context = default.context,
            TasteField::Tier => self.tier = default.tier,
            TasteField::DefaultModel => self.default_model = default.default_model,
            TasteField::DefaultEffort => self.default_effort = default.default_effort,
            TasteField::MaxEffort => self.max_effort = default.max_effort,
            TasteField::BudgetCents => self.budget_cents = default.budget_cents,
            TasteField::TimeLimitMin => self.time_limit_min = default.time_limit_min,
            TasteField::BoxBudgetCents => self.box_budget_cents = default.box_budget_cents,
            TasteField::Placement | TasteField::BoxesAtOnce => {}
            TasteField::BoxSize => self.box_size = default.box_size,
            TasteField::EscalateBreakingChange => self.escalate_breaking_change = default.escalate_breaking_change,
            TasteField::EscalateProcessFailure => self.escalate_process_failure = default.escalate_process_failure,
            TasteField::EscalateOverLines => self.escalate_over_lines = default.escalate_over_lines,
            TasteField::EscalateAreas => self.escalate_areas = default.escalate_areas,
            TasteField::ManageOnly => self.manage_only = default.manage_only,
            TasteField::ClaudeMemory => self.claude_memory = default.claude_memory,
            TasteField::WorkerClaudeMemory => self.worker_claude_memory = default.worker_claude_memory,
        }
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Taste::default().limits()
    }
}

const RAISE: &str = "in Settings › Way of working";

impl Limits {
    /// The tighter of each limit, so a worker's own workers never get more than it has.
    pub fn narrow(self, outer: Limits) -> Limits {
        let cap = |a: u32, b: u32| if a == 0 || b == 0 { a.max(b) } else { a.min(b) };
        let max_effort = self.max_effort.min(outer.max_effort);
        Limits {
            parallel: self.parallel.min(outer.parallel),
            tier: self.tier.min(outer.tier),
            default_effort: self.default_effort.or(outer.default_effort).map(|effort| effort.min(max_effort)),
            max_effort,
            budget_cents: cap(self.budget_cents, outer.budget_cents),
            time_limit_min: self.time_limit_min.min(outer.time_limit_min),
            box_budget_cents: cap(self.box_budget_cents, outer.box_budget_cents),
        }
    }

    /// The effort a worker on `model` runs at: the one asked for, or the default. Refuses a model
    /// above the tier, a call that names no model (or an empty one) under a tier cap, and an effort above the most.
    /// `claude` is the machine's Claude models, so a model its claude's menu added has its tier.
    /// On a Claude login the tier of a model is the one `claude` lists for it, so a model id that
    /// list does not hold is refused under a tier cap: the words of an id are no proof of its cost.
    /// An effort outside the five levels, such as Cursor's `fast`, passes unranked.
    pub fn admit(&self, account: &crate::accounts::Account, model: Option<&str>, effort: Option<&str>, claude: &[crate::claude_models::ClaudeModel]) -> Result<Option<String>, String> {
        let lists_claude = account.provider == crate::accounts::Provider::Claude && account.auth == crate::accounts::Auth::Login;
        self.admit_priced(lists_claude, model, effort, claude)
    }

    /// `admit` for an account this machine does not hold, whose login and list of models are not
    /// known here: the model is priced by its words, as `admit` does for the other drivers. A
    /// `claude-` id is still held to this machine's Claude list, since a machine on 0.18.1 that
    /// receives it prices by words alone and would let an id with no cost word through as Mid.
    pub fn admit_unheld(&self, model: Option<&str>, effort: Option<&str>, claude: &[crate::claude_models::ClaudeModel]) -> Result<Option<String>, String> {
        let claude_id = model.is_some_and(|m| m.starts_with("claude-"));
        self.admit_priced(claude_id, model, effort, claude)
    }

    fn admit_priced(&self, lists_claude: bool, model: Option<&str>, effort: Option<&str>, claude: &[crate::claude_models::ClaudeModel]) -> Result<Option<String>, String> {
        if self.tier < Tier::High {
            let tier = self.tier.name();
            let Some(model) = model.filter(|m| !m.is_empty()) else {
                return Err(format!("name a model: this project allows models up to the {tier} tier, and a worker without one runs on its account's default. Pick a {tier} model from orchestrator_capabilities, or raise Tier {RAISE}"));
            };
            if lists_claude && !claude.iter().any(|m| m.slug == model) {
                return Err(format!("{model} is not in this machine's list of Claude models, so its tier is unknown and this project allows up to {tier}. Pick a model from orchestrator_capabilities or raise Tier {RAISE}"));
            }
            let cost = crate::delegate::cost_in(claude, model);
            if cost > self.tier {
                return Err(format!("{model} is a {}-tier model; this project allows up to {tier}. Pick a {tier} model or raise Tier {RAISE}", cost.name()));
            }
        }
        let Some(asked) = effort else { return Ok(self.default_effort.map(|effort| effort.name().to_owned())) };
        if Effort::parse(asked).is_some_and(|level| level > self.max_effort) {
            let max = self.max_effort.name();
            return Err(format!("effort {asked} is above {max}, the most this project allows. Ask for {max} or less, or raise Max effort {RAISE}"));
        }
        Ok(Some(asked.to_owned()))
    }

    pub fn time_limit_ms(&self) -> u64 {
        u64::from(self.time_limit_min) * 60_000
    }

    /// Why a worker that spent `cost_usd` must stop, if it passed the budget — or one whose spend
    /// can't be known at all: a cap that can't be seen can't hold, so it stops instead of running
    /// free in silence. `model` is the one it ran on, when its record names it.
    pub fn over_budget(&self, cost_usd: Option<f64>, model: Option<&str>) -> Option<String> {
        if self.budget_cents == 0 {
            return None;
        }
        let Some(spent) = cost_usd else {
            let model = model.filter(|m| !m.is_empty()).unwrap_or("its model");
            return Some(format!("{model} has no price, so pinned can't hold it to its budget of {}. Set one in Settings › Usage › Model prices, or set the budget to 0", usd(self.budget_cents)));
        };
        (spent * 100.0 >= f64::from(self.budget_cents)).then(|| format!("pinned stopped it after it spent US$ {spent:.2}, its budget of {}", usd(self.budget_cents)))
    }

    /// A box's budget in USD: the one asked for, at most the box budget, which is also the default.
    pub fn box_budget_usd(&self, asked: Option<f64>) -> Result<Option<f64>, String> {
        if self.box_budget_cents == 0 {
            return Ok(asked);
        }
        let ceiling = f64::from(self.box_budget_cents) / 100.0;
        match asked {
            Some(usd) if usd > ceiling + 1e-9 => Err(format!("a box may spend at most {}; name less, or raise Box budget {RAISE}", self::usd(self.box_budget_cents))),
            Some(usd) => Ok(Some(usd)),
            None => Ok(Some(ceiling)),
        }
    }

    /// What a session that delegates is told about the limits pinned enforces.
    pub fn hint(&self) -> String {
        let effort = match self.default_effort {
            Some(default) => format!("effort up to {} ({} when a call names none)", self.max_effort.name(), default.name()),
            None => format!("effort up to {}", self.max_effort.name()),
        };
        let mut limits = vec![
            match self.parallel {
                1 => "one worker at a time".to_owned(),
                n => format!("at most {n} workers at once"),
            },
            format!("models up to the {} tier", self.tier.name()),
            effort,
            format!("{} minutes per worker", self.time_limit_min),
        ];
        if self.budget_cents > 0 {
            limits.push(format!("{} per worker", usd(self.budget_cents)));
        }
        if self.box_budget_cents > 0 {
            limits.push(format!("{} per box", usd(self.box_budget_cents)));
        }
        format!("pinned enforces these delegation limits on delegate_task: {}. A call past them is refused, and a worker past its time or budget is stopped; if a limit blocks you, tell the user to raise it {RAISE} rather than working around it. The tier covers the model each worker runs on, not the models it picks for its own subagents.", limits.join(", "))
    }
}

pub(crate) fn usd(cents: u32) -> String {
    format!("US$ {}.{:02}", cents / 100, cents % 100)
}

impl TastePatch {
    /// The settings this patch sets that only the user changes, on the Way of working page: the review settings,
    /// Boxes at once, the escalation settings and Manage only, by name.
    pub fn review_settings(&self) -> Vec<&'static str> {
        [
            ("review_diffs", self.review_diffs.is_some()),
            ("second_opinion", self.second_opinion.is_some()),
            ("boxes_at_once", self.boxes_at_once.is_some()),
            ("escalate_breaking_change", self.escalate_breaking_change.is_some()),
            ("escalate_process_failure", self.escalate_process_failure.is_some()),
            ("escalate_over_lines", self.escalate_over_lines.is_some()),
            ("escalate_areas", self.escalate_areas.is_some()),
            ("manage_only", self.manage_only.is_some()),
            ("claude_memory", self.claude_memory.is_some()),
            ("worker_claude_memory", self.worker_claude_memory.is_some()),
        ]
            .into_iter()
            .filter(|(_, set)| *set)
            .map(|(name, _)| name)
            .collect()
    }

    /// Overwrites each of `taste`'s knobs this patch sets.
    pub fn apply(&self, taste: &mut Taste) {
        fn put<T: Clone>(set: &Option<T>, knob: &mut T) {
            if let Some(value) = set {
                *knob = value.clone();
            }
        }
        put(&self.strategy, &mut taste.strategy);
        put(&self.ladder, &mut taste.ladder);
        if self.hard_model.is_some() {
            taste.hard_model.clone_from(&self.hard_model);
        }
        put(&self.parallel, &mut taste.parallel);
        put(&self.autonomy, &mut taste.autonomy);
        put(&self.review_diffs, &mut taste.review_diffs);
        put(&self.second_opinion, &mut taste.second_opinion);
        put(&self.skills, &mut taste.skills);
        put(&self.context, &mut taste.context);
        put(&self.tier, &mut taste.tier);
        if self.default_model.is_some() {
            taste.default_model.clone_from(&self.default_model);
        }
        if self.default_effort.is_some() {
            taste.default_effort = self.default_effort;
        }
        put(&self.max_effort, &mut taste.max_effort);
        put(&self.budget_cents, &mut taste.budget_cents);
        put(&self.time_limit_min, &mut taste.time_limit_min);
        put(&self.box_budget_cents, &mut taste.box_budget_cents);
        if self.box_size.is_some() {
            taste.box_size = self.box_size;
        }
        put(&self.escalate_breaking_change, &mut taste.escalate_breaking_change);
        put(&self.escalate_process_failure, &mut taste.escalate_process_failure);
        put(&self.escalate_over_lines, &mut taste.escalate_over_lines);
        put(&self.escalate_areas, &mut taste.escalate_areas);
        put(&self.manage_only, &mut taste.manage_only);
        put(&self.claude_memory, &mut taste.claude_memory);
        put(&self.worker_claude_memory, &mut taste.worker_claude_memory);
    }

    /// Takes each field `newer` sets, and keeps the rest.
    pub fn merge(&mut self, newer: TastePatch) {
        fn take<T>(newer: Option<T>, field: &mut Option<T>) {
            if newer.is_some() {
                *field = newer;
            }
        }
        take(newer.strategy, &mut self.strategy);
        take(newer.ladder, &mut self.ladder);
        take(newer.hard_model, &mut self.hard_model);
        take(newer.parallel, &mut self.parallel);
        take(newer.autonomy, &mut self.autonomy);
        take(newer.review_diffs, &mut self.review_diffs);
        take(newer.second_opinion, &mut self.second_opinion);
        take(newer.skills, &mut self.skills);
        take(newer.context, &mut self.context);
        take(newer.tier, &mut self.tier);
        take(newer.default_model, &mut self.default_model);
        take(newer.default_effort, &mut self.default_effort);
        take(newer.max_effort, &mut self.max_effort);
        take(newer.budget_cents, &mut self.budget_cents);
        take(newer.time_limit_min, &mut self.time_limit_min);
        take(newer.box_budget_cents, &mut self.box_budget_cents);
        take(newer.box_size, &mut self.box_size);
        take(newer.placement, &mut self.placement);
        take(newer.boxes_at_once, &mut self.boxes_at_once);
        take(newer.escalate_breaking_change, &mut self.escalate_breaking_change);
        take(newer.escalate_process_failure, &mut self.escalate_process_failure);
        take(newer.escalate_over_lines, &mut self.escalate_over_lines);
        take(newer.escalate_areas, &mut self.escalate_areas);
        take(newer.manage_only, &mut self.manage_only);
        take(newer.claude_memory, &mut self.claude_memory);
        take(newer.worker_claude_memory, &mut self.worker_claude_memory);
    }

    /// Drops each field equal to `base`'s, so the scope inherits it again.
    pub fn prune(&mut self, base: &Taste) {
        fn drop_same<T: PartialEq>(field: &mut Option<T>, base: &T) {
            if field.as_ref() == Some(base) {
                *field = None;
            }
        }
        drop_same(&mut self.strategy, &base.strategy);
        drop_same(&mut self.ladder, &base.ladder);
        if self.hard_model.is_some() && self.hard_model == base.hard_model {
            self.hard_model = None;
        }
        drop_same(&mut self.parallel, &base.parallel);
        drop_same(&mut self.autonomy, &base.autonomy);
        drop_same(&mut self.review_diffs, &base.review_diffs);
        drop_same(&mut self.second_opinion, &base.second_opinion);
        drop_same(&mut self.skills, &base.skills);
        drop_same(&mut self.context, &base.context);
        drop_same(&mut self.tier, &base.tier);
        if self.default_model.is_some() && self.default_model == base.default_model {
            self.default_model = None;
        }
        if self.default_effort.is_some() && self.default_effort == base.default_effort {
            self.default_effort = None;
        }
        drop_same(&mut self.max_effort, &base.max_effort);
        drop_same(&mut self.budget_cents, &base.budget_cents);
        drop_same(&mut self.time_limit_min, &base.time_limit_min);
        drop_same(&mut self.box_budget_cents, &base.box_budget_cents);
        if self.box_size.is_some() && self.box_size == base.box_size {
            self.box_size = None;
        }
        drop_same(&mut self.escalate_breaking_change, &base.escalate_breaking_change);
        drop_same(&mut self.escalate_process_failure, &base.escalate_process_failure);
        drop_same(&mut self.escalate_over_lines, &base.escalate_over_lines);
        drop_same(&mut self.escalate_areas, &base.escalate_areas);
        drop_same(&mut self.manage_only, &base.manage_only);
        drop_same(&mut self.claude_memory, &base.claude_memory);
        drop_same(&mut self.worker_claude_memory, &base.worker_claude_memory);
    }

    pub fn is_empty(&self) -> bool {
        *self == TastePatch::default()
    }

    /// Whether this patch sets only `parallel`, to the value `base`'s preset had before the
    /// defaults rose. Such a patch was the old default, not a choice, so its project inherits the
    /// base's new value again.
    fn sets_only_old_parallel(&self, base: Profile) -> bool {
        TastePatch { parallel: self.parallel, ..TastePatch::default() } == *self
            && OLD_PRESET_PARALLEL.iter().any(|(profile, old)| *profile == base && self.parallel == Some(*old))
    }

    fn clear(&mut self, field: TasteField) {
        match field {
            TasteField::Strategy => self.strategy = None,
            TasteField::Ladder => self.ladder = None,
            TasteField::HardModel => self.hard_model = None,
            TasteField::Parallel => self.parallel = None,
            TasteField::Autonomy => self.autonomy = None,
            TasteField::ReviewDiffs => self.review_diffs = None,
            TasteField::SecondOpinion => self.second_opinion = None,
            TasteField::Skills => self.skills = None,
            TasteField::Context => self.context = None,
            TasteField::Tier => self.tier = None,
            TasteField::DefaultModel => self.default_model = None,
            TasteField::DefaultEffort => self.default_effort = None,
            TasteField::MaxEffort => self.max_effort = None,
            TasteField::BudgetCents => self.budget_cents = None,
            TasteField::TimeLimitMin => self.time_limit_min = None,
            TasteField::BoxBudgetCents => self.box_budget_cents = None,
            TasteField::Placement => self.placement = None,
            TasteField::BoxSize => self.box_size = None,
            TasteField::BoxesAtOnce => self.boxes_at_once = None,
            TasteField::EscalateBreakingChange => self.escalate_breaking_change = None,
            TasteField::EscalateProcessFailure => self.escalate_process_failure = None,
            TasteField::EscalateOverLines => self.escalate_over_lines = None,
            TasteField::EscalateAreas => self.escalate_areas = None,
            TasteField::ManageOnly => self.manage_only = None,
            TasteField::ClaudeMemory => self.claude_memory = None,
            TasteField::WorkerClaudeMemory => self.worker_claude_memory = None,
        }
    }
}

impl Tastes {
    /// Reads tastes stored before the parallel defaults rose as what they were saved as: each
    /// whole taste that was exactly a preset takes that preset's new workers at once, and a
    /// project patch that set only the old default over its preset's base inherits the base
    /// again. A knob the account really chose stays as it is.
    pub fn take_raised_defaults(&mut self) {
        self.orchestrator.raise_parallel();
        self.captains.raise_parallel();
        let base = self.captains.profile();
        self.projects.retain(|_, patch| !patch.sets_only_old_parallel(base));
    }

    pub fn resolve(&self, scope: &TasteScope) -> Taste {
        match scope {
            TasteScope::Orchestrator => self.orchestrator.clone(),
            TasteScope::Captains => self.captains.clone(),
            TasteScope::Project { repo } => {
                let mut taste = self.captains.clone();
                if let Some(patch) = self.projects.get(repo) {
                    patch.apply(&mut taste);
                }
                taste
            }
        }
    }

    /// Applies `patch` to the scope, or changes nothing when the result fails `Taste::check`. A
    /// project's patch keeps only the fields that differ from the Capitães'. Each escalation area is
    /// trimmed, and a blank one dropped.
    pub fn set(&mut self, scope: &TasteScope, mut patch: TastePatch) -> Result<(), String> {
        if let Some(areas) = &mut patch.escalate_areas {
            *areas = areas.iter().map(|area| area.trim()).filter(|area| !area.is_empty()).map(str::to_owned).collect();
        }
        if let Some(boxes) = patch.boxes_at_once.take() {
            if matches!(scope, TasteScope::Project { .. }) {
                return Err("Boxes at once belongs to the account: set it for Captains (all), not for one project".into());
            }
            let mut changed = Tastes { boxes_at_once: boxes, ..self.clone() };
            if !patch.is_empty() {
                changed.set(scope, patch)?;
            }
            *self = changed;
            return Ok(());
        }
        if let Some(placement) = patch.placement.take() {
            if matches!(scope, TasteScope::Project { .. }) {
                return Err("placement belongs to the account: set it for Captains (all), not for one project".into());
            }
            placement.check()?;
            if patch.is_empty() {
                self.placement = placement;
                return Ok(());
            }
            let mut changed = self.clone();
            changed.placement = placement;
            changed.set(scope, patch)?;
            *self = changed;
            return Ok(());
        }
        if patch.box_size.is_some() && *scope == TasteScope::Orchestrator {
            return Err("the box size belongs to the projects: set the account's for Captains (all), or one project's".into());
        }
        let taste = match scope {
            TasteScope::Orchestrator => &mut self.orchestrator,
            TasteScope::Captains => &mut self.captains,
            TasteScope::Project { repo } => return self.set_project(repo, patch),
        };
        let mut changed = taste.clone();
        patch.apply(&mut changed);
        changed.check()?;
        *taste = changed;
        Ok(())
    }

    fn set_project(&mut self, repo: &str, patch: TastePatch) -> Result<(), String> {
        if repo.trim().is_empty() {
            return Err("a project's taste needs the project's repo".into());
        }
        let mut merged = self.projects.get(repo).cloned().unwrap_or_default();
        merged.merge(patch);
        merged.prune(&self.captains);
        let mut resolved = self.captains.clone();
        merged.apply(&mut resolved);
        resolved.check()?;
        if merged.is_empty() {
            self.projects.remove(repo);
        } else {
            self.projects.insert(repo.to_owned(), merged);
        }
        Ok(())
    }

    /// A project inherits the field again; the orchestrator and the Capitães get its default back.
    pub fn reset(&mut self, scope: &TasteScope, field: TasteField) {
        if field == TasteField::Placement && !matches!(scope, TasteScope::Project { .. }) {
            self.placement = crate::placement::Placement::default();
            return;
        }
        if field == TasteField::BoxesAtOnce && !matches!(scope, TasteScope::Project { .. }) {
            self.boxes_at_once = DEFAULT_BOXES_AT_ONCE;
            return;
        }
        match scope {
            TasteScope::Orchestrator => self.orchestrator.reset(field),
            TasteScope::Captains => self.captains.reset(field),
            TasteScope::Project { repo } => {
                if let Some(patch) = self.projects.get_mut(repo) {
                    patch.clear(field);
                    if patch.is_empty() {
                        self.projects.remove(repo);
                    }
                }
            }
        }
    }

    /// Whether every scope, each project resolved, passes `Taste::check`, the Reviewer passes
    /// `Reviewer::check`, and every list of worker profiles passes the checks of its own write.
    pub fn check(&self) -> Result<(), String> {
        self.orchestrator.check()?;
        self.captains.check()?;
        self.placement.check()?;
        self.projects.keys().try_for_each(|repo| self.resolve(&TasteScope::Project { repo: repo.clone() }).check())?;
        self.reviewer.check()?;
        let profiles = &self.worker_profiles;
        std::iter::once(&profiles.account).chain(profiles.projects.values()).chain([&profiles.proposed]).try_for_each(|list| check_profiles(list))?;
        if profiles.proposed.len() > MAX_PROPOSED {
            return Err(too_many_proposed());
        }
        Ok(())
    }

    /// `HubCall::ProposeWorkerProfile`: `profile`, its name trimmed and written as an
    /// orchestrator's, waits in `proposed` for the user, in place of a proposal of the same name.
    /// Nothing outside `proposed` changes, and nothing at all when it fails `WorkerProfile::check`
    /// or `MAX_PROPOSED` others already wait.
    pub fn propose_worker_profile(&mut self, mut profile: WorkerProfile) -> Result<(), String> {
        profile.name = profile.name.trim().to_owned();
        profile.by = ProfileAuthor::Orchestrator;
        profile.check()?;
        let proposed = &mut self.worker_profiles.proposed;
        match proposed.iter().position(|p| same_name(&p.name, &profile.name)) {
            Some(at) => proposed[at] = profile,
            None if proposed.len() >= MAX_PROPOSED => return Err(too_many_proposed()),
            None => proposed.push(profile),
        }
        Ok(())
    }

    /// Removes the proposals named in `names`, as `HubCall::SetWorkerProfiles` drops them.
    pub fn drop_proposed(&mut self, names: &[String]) {
        self.worker_profiles.proposed.retain(|p| !names.iter().any(|name| same_name(&p.name, name)));
    }
}

/// The most proposed worker profiles that wait for the user at once.
pub const MAX_PROPOSED: usize = 20;

fn too_many_proposed() -> String {
    format!("{MAX_PROPOSED} proposed profiles already wait for the user to take or drop; nothing changed")
}

/// Profile names match trimmed and without case, as `Tastes::set_worker_profiles` compares them.
pub(crate) fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Each profile passes `WorkerProfile::check` and no two share a name.
fn check_profiles(list: &[WorkerProfile]) -> Result<(), String> {
    let mut names = std::collections::BTreeSet::new();
    for profile in list {
        profile.check()?;
        if !names.insert(profile.name.trim().to_lowercase()) {
            return Err(format!("two profiles are named {}", profile.name.trim()));
        }
    }
    Ok(())
}

/// Which setting chose a box's size, as the app names it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BoxSizeSource {
    /// The call that opens the box named it.
    Call,
    /// The project's Way of working.
    Project,
    /// The account's Way of working, under Captains (all).
    Account,
    /// Nothing named one: small.
    BuiltIn,
}

/// The size a box opens at, and which setting chose it. Only `Tastes::box_size` makes one, so a
/// path that opens a box cannot pick a size around the project's and the account's defaults.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxSizeChoice {
    size: BoxSize,
    from: BoxSizeSource,
}

impl BoxSizeChoice {
    pub fn size(self) -> BoxSize {
        self.size
    }

    pub fn from(self) -> BoxSizeSource {
        self.from
    }
}

impl Tastes {
    /// The size of a box that clones `repo`, in any form `machine::repo_key` reads: the size the
    /// call names, else the project's default box size, else the account's, else small.
    pub fn box_size(&self, repo: Option<&str>, asked: Option<BoxSize>) -> BoxSizeChoice {
        let project = repo.and_then(crate::machine::repo_key).and_then(|key| self.projects.get(&key)).and_then(|patch| patch.box_size);
        let (size, from) = match (asked, project, self.captains.box_size) {
            (Some(size), _, _) => (size, BoxSizeSource::Call),
            (None, Some(size), _) => (size, BoxSizeSource::Project),
            (None, None, Some(size)) => (size, BoxSizeSource::Account),
            (None, None, None) => (BoxSize::default(), BoxSizeSource::BuiltIn),
        };
        BoxSizeChoice { size, from }
    }
}

impl TasteScope {
    /// `orchestrator`, `captains` or `project:<repo>`, as the taste tools name a scope.
    pub fn parse(text: &str) -> Result<TasteScope, String> {
        match text.trim() {
            "orchestrator" => Ok(TasteScope::Orchestrator),
            "captains" => Ok(TasteScope::Captains),
            other => match other.strip_prefix("project:").map(str::trim) {
                Some(repo) if !repo.is_empty() => Ok(TasteScope::Project { repo: repo.to_owned() }),
                _ => Err(format!("unknown scope {other:?}: use orchestrator, captains or project:<repo>, as in project:github.com/acme/site")),
            },
        }
    }

    pub fn name(&self) -> String {
        match self {
            TasteScope::Orchestrator => "orchestrator".into(),
            TasteScope::Captains => "captains".into(),
            TasteScope::Project { repo } => format!("project:{repo}"),
        }
    }

    /// Who the scope's sessions are, as the Way of working page names its tab.
    pub fn label(&self) -> String {
        match self {
            TasteScope::Orchestrator => "the global orchestrator".into(),
            TasteScope::Captains => "every project's Captain (Captains (all))".into(),
            TasteScope::Project { repo } => format!("the Captain of the project {repo}"),
        }
    }
}

/// What `taste_get` answers about one scope.
#[derive(Serialize, Debug, PartialEq)]
pub struct ScopeView {
    pub scope: String,
    pub profile: Profile,
    /// Every knob as the scope's sessions get it.
    pub effective: Taste,
    /// What the scope sets itself: a project's own fields, or the whole taste of the others.
    pub own: Value,
    /// A project inherits each knob it leaves out from here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inherits_from: Option<String>,
    /// The text the scope's sessions start with.
    pub brief: String,
    /// The account's `USER_ONLY` settings, which the taste tools read and never change.
    pub reviewer: crate::reviewer::Reviewer,
    pub worker_profiles: crate::profiles::WorkerProfiles,
}

impl Tastes {
    pub fn view(&self, scope: &TasteScope) -> ScopeView {
        let effective = self.resolve(scope);
        let own = match scope {
            TasteScope::Orchestrator => serde_json::to_value(&self.orchestrator),
            TasteScope::Captains => serde_json::to_value(&self.captains),
            TasteScope::Project { repo } => serde_json::to_value(self.projects.get(repo).cloned().unwrap_or_default()),
        }
        .unwrap_or_default();
        ScopeView {
            scope: scope.name(),
            profile: effective.profile(),
            brief: effective.brief(),
            own,
            inherits_from: matches!(scope, TasteScope::Project { .. }).then(|| TasteScope::Captains.name()),
            effective,
            reviewer: self.reviewer.clone(),
            worker_profiles: self.worker_profiles.clone(),
        }
    }
}

/// `PINNED_ROLE` of a thread opened to adjust the way of working by talking. It is not a `Role`, so
/// an older app shows it as a plain thread.
pub const CHAT_ROLE: &str = "taste-chat";

pub fn chat_env() -> Vec<(String, String)> {
    vec![(crate::agent::ROLE_ENV.to_owned(), CHAT_ROLE.to_owned())]
}

pub fn is_chat(env: &[(String, String)]) -> bool {
    env.iter().any(|(k, v)| k == crate::agent::ROLE_ENV && v == CHAT_ROLE)
}

/// The first prompt of a thread that adjusts `scope`'s way of working.
pub fn chat_prompt(scope: &TasteScope) -> String {
    let name = scope.name();
    format!(
        "Help me adjust the way of working of {who} in pinned (scope {name}).

You have three tools for it. taste_get(scope) reads a scope: the settings its sessions get (effective), what the scope sets itself (own), and the brief text they start with. taste_set(scope, patch) changes the fields the patch names and keeps the rest; a project's field set to the Captains' value goes back to inheriting it. taste_reset(scope, field) makes a project inherit the field again, or gives the orchestrator and the Captains its default back. A scope is orchestrator, captains, or project: followed by the project's repo, as in project:github.com/acme/site. The fields are strategy (cheap_with_guidance, escalate_on_failure or strongest_first), ladder (models, cheapest first), hard_model, parallel (workers at once, 1 to 100), autonomy (ask_first, decide_and_tell or report_at_end), skills ({{name, enabled}}), context (notes every turn carries) and default_model (a model, with its provider when that matters: the model new sessions start on when they name none, on a Claude Code or OpenCode Go account that lists it), and the delegation limits pinned enforces: tier (low, mid or high), default_effort and max_effort (low, medium, high, xhigh or max; default_effort is the effort of a worker whose call names none and of new sessions that name none), budget_cents (per worker, 0 for no cap), time_limit_min (1 to 10080) and box_budget_cents (per box, 0 for none), and box_size (small, medium or large: the size of a box opened with none named; captains holds the account's, a project overrides it, small when neither does). Leave review_diffs, second_opinion, boxes_at_once, manage_only, claude_memory, worker_claude_memory and the escalation settings (escalate_breaking_change, escalate_process_failure, escalate_over_lines and escalate_areas) alone: taste_set and taste_reset refuse them, and only the user sets them, on the Way of working page. The Way of working page shows each change as soon as it lands, and new sessions start with it.

Start with taste_get(\"{name}\"). Then interview me: one short question at a time, each with two or three answers to pick from, and change nothing yet. When you know what to change, list the changes in plain words, apply them with taste_set or taste_reset once I agree, and read the scope again to show me the new brief.",
        who = scope.label(),
    )
}

/// The tastes in `dir`'s `taste.json`, read as what they were saved as on the current parallel
/// defaults. Without one, an `orchestration.json` from before tastes existed seeds the Capitães
/// with its cost style's preset, written to `taste.json` so the seed outlives the style. None when
/// neither is there, or when `taste.json` can't be read, which the seed must not overwrite.
pub fn load(dir: Option<&Path>) -> Option<Tastes> {
    let dir = dir?;
    match std::fs::read(dir.join(FILE)) {
        Ok(bytes) => {
            return serde_json::from_slice(&bytes)
                .map_err(|e| eprintln!("pinned: {} is unreadable, so the default taste applies: {e}", dir.join(FILE).display()))
                .map(|mut tastes: Tastes| {
                    tastes.take_raised_defaults();
                    tastes
                })
                .ok();
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            eprintln!("pinned: could not read {}: {e}", dir.join(FILE).display());
            return None;
        }
        Err(_) => {}
    }
    let old: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(OLD_FILE)).ok()?).ok()?;
    let profile = match old["style"].as_str()? {
        "economical" => Profile::Economical,
        "balanced" => Profile::Balanced,
        "quality" => Profile::Aggressive,
        _ => return None,
    };
    let tastes = Tastes { captains: profile.preset()?, ..Tastes::default() };
    if let Err(e) = save(&tastes, Some(dir)) {
        eprintln!("pinned: could not save the taste seeded from the cost style: {e}");
    }
    Some(tastes)
}

/// Moves the old per-machine worker cap of `orchestration.json` into the tastes once: a cap below
/// a scope's workers at once lowers it there, then the file goes. A machine linked to an account
/// that already has tastes on the hub keeps the hub's; the cap reaches the hub only with the rest
/// of this machine's tastes, when the account has none yet.
pub fn migrate_worker_cap(dir: Option<&Path>) {
    let Some(dir) = dir else { return };
    let file = dir.join(OLD_FILE);
    let Ok(bytes) = std::fs::read(&file) else { return };
    load(Some(dir));
    let cap = serde_json::from_slice::<serde_json::Value>(&bytes).ok().and_then(|old| old["max_workers"].as_u64()).map(|cap| cap.clamp(1, u64::from(MAX_PARALLEL)) as u32);
    if let Some(cap) = cap.filter(|cap| *cap < MAX_PARALLEL) {
        let lowered = change(Some(dir), |tastes| {
            for taste in [&mut tastes.orchestrator, &mut tastes.captains] {
                taste.parallel = taste.parallel.min(cap);
            }
            for patch in tastes.projects.values_mut() {
                patch.parallel = patch.parallel.map(|parallel| parallel.min(cap));
            }
            Ok(())
        });
        if let Err(e) = lowered {
            eprintln!("pinned: could not move the worker cap of {} into the taste: {e}", file.display());
            return;
        }
    }
    if let Err(e) = std::fs::remove_file(&file) {
        eprintln!("pinned: could not remove {}: {e}", file.display());
    }
}

/// Reads `dir`'s tastes, applies `change` and writes them back, one change at a time. Nothing is
/// written when `change` fails.
pub fn change(dir: Option<&Path>, change: impl FnOnce(&mut Tastes) -> Result<(), String>) -> Result<Tastes, String> {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut tastes = load(dir).unwrap_or_default();
    change(&mut tastes)?;
    save(&tastes, dir)?;
    Ok(tastes)
}

/// Writes `taste.json` whole or not at all, so a reader never sees half of it.
pub fn save(tastes: &Tastes, dir: Option<&Path>) -> Result<(), String> {
    let Some(dir) = dir else { return Ok(()) };
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let partial = dir.join(format!("{FILE}.{}.{:?}", std::process::id(), std::thread::current().id()));
    std::fs::write(&partial, serde_json::to_vec_pretty(tastes).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, dir.join(FILE)).map_err(|e| e.to_string())
}

impl Tastes {
    /// One item per orchestrator and Captains field, per project patch, plus placement, the
    /// Reviewer and the worker profiles. An empty project patch is absent: the project inherits.
    pub fn copy_items(&self) -> BTreeMap<crate::account_copy::Item, crate::account_copy::Version> {
        use crate::account_copy::{Item, Version};
        let mut items = BTreeMap::new();
        let bare = |value: Value| Version { value, secrets: BTreeMap::new() };
        for (scope, taste) in [("orchestrator", &self.orchestrator), ("captains", &self.captains)] {
            let Ok(value) = serde_json::to_value(taste) else { continue };
            let Some(object) = value.as_object() else { continue };
            for field in crate::account_copy::TASTE_FIELDS {
                if let Some(field_value) = object.get(*field) {
                    items.insert(Item::taste(&format!("{scope}/{field}")), bare(field_value.clone()));
                }
            }
        }
        for (repo, patch) in &self.projects {
            if patch.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::to_value(patch) {
                items.insert(Item::taste(&format!("project/{repo}")), bare(value));
            }
        }
        if let Ok(value) = serde_json::to_value(&self.placement) {
            items.insert(Item::taste("placement"), bare(value));
        }
        if let Ok(value) = serde_json::to_value(&self.reviewer) {
            items.insert(Item::taste("reviewer"), bare(value));
        }
        if let Ok(value) = serde_json::to_value(&self.worker_profiles) {
            items.insert(Item::taste("worker-profiles"), bare(value));
        }
        items
    }

    /// Writes each taste item onto this account. Items that are not tastes are left alone. A project
    /// whose patch is empty is removed, so it inherits again.
    pub fn apply_copy_items(&mut self, items: &BTreeMap<crate::account_copy::Item, crate::account_copy::Version>) -> Result<(), String> {
        let mut orchestrator = serde_json::to_value(&self.orchestrator).map_err(|e| e.to_string())?;
        let mut captains = serde_json::to_value(&self.captains).map_err(|e| e.to_string())?;
        for (item, version) in items {
            if !item.is_taste() {
                continue;
            }
            let path = item.as_str().trim_start_matches("taste/");
            if let Some(field) = path.strip_prefix("orchestrator/") {
                set_taste_field(&mut orchestrator, field, version.value.clone())?;
            } else if let Some(field) = path.strip_prefix("captains/") {
                set_taste_field(&mut captains, field, version.value.clone())?;
            } else if let Some(repo) = path.strip_prefix("project/") {
                let patch: TastePatch = serde_json::from_value(version.value.clone()).map_err(|e| e.to_string())?;
                if patch.is_empty() {
                    self.projects.remove(repo);
                } else {
                    self.projects.insert(repo.to_owned(), patch);
                }
            } else if path == "placement" {
                self.placement = serde_json::from_value(version.value.clone()).map_err(|e| e.to_string())?;
            } else if path == "reviewer" {
                self.reviewer = serde_json::from_value(version.value.clone()).map_err(|e| e.to_string())?;
            } else if path == "worker-profiles" {
                self.worker_profiles = serde_json::from_value(version.value.clone()).map_err(|e| e.to_string())?;
            }
        }
        self.orchestrator = serde_json::from_value(orchestrator).map_err(|e| e.to_string())?;
        self.captains = serde_json::from_value(captains).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// These tastes with the items written on, and every project the copy does not name gone. A
    /// setting the copy does not carry, such as the escalation settings or Boxes at once, keeps its
    /// value here.
    pub fn with_copy_items(&self, items: &BTreeMap<crate::account_copy::Item, crate::account_copy::Version>) -> Result<Tastes, String> {
        let mut tastes = Tastes { projects: BTreeMap::new(), ..self.clone() };
        tastes.apply_copy_items(items)?;
        Ok(tastes)
    }

    /// The tastes those items are, starting from the defaults so a project the copy does not name
    /// is gone.
    pub fn from_copy_items(items: &BTreeMap<crate::account_copy::Item, crate::account_copy::Version>) -> Result<Tastes, String> {
        let mut tastes = Tastes::default();
        tastes.apply_copy_items(items)?;
        Ok(tastes)
    }

    /// The item as the current taste types read it. A value that does not read stays as it arrived.
    pub fn normalize_copy_item(item: &crate::account_copy::Item, version: &crate::account_copy::Version) -> crate::account_copy::Version {
        let mut items = BTreeMap::new();
        items.insert(item.clone(), version.clone());
        let Ok(tastes) = Tastes::from_copy_items(&items) else { return version.clone() };
        let Some(found) = tastes.copy_items().remove(item) else { return version.clone() };
        crate::account_copy::Version { secrets: version.secrets.clone(), ..found }
    }
}

fn set_taste_field(taste: &mut Value, field: &str, value: Value) -> Result<(), String> {
    taste.as_object_mut().ok_or_else(|| "a taste is an object".to_owned())?.insert(field.to_owned(), value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SITE: &str = "github.com/acme/site";

    #[test]
    fn copy_items_round_trip_a_field_a_project_and_the_account_pieces() {
        let mut tastes = Tastes::default();
        tastes.set(&TasteScope::Captains, TastePatch { parallel: Some(2), ..TastePatch::default() }).unwrap();
        tastes.set(&site(), TastePatch { parallel: Some(3), ..TastePatch::default() }).unwrap();
        let items = tastes.copy_items();
        let captains = crate::account_copy::Item::try_from("taste/captains/parallel".to_owned()).unwrap();
        let project = crate::account_copy::Item::try_from(format!("taste/project/{SITE}")).unwrap();
        assert_eq!(items.get(&captains).unwrap().value, json!(2));
        assert!(items.contains_key(&project));
        assert!(items.contains_key(&crate::account_copy::Item::try_from("taste/placement".to_owned()).unwrap()));
        assert!(items.contains_key(&crate::account_copy::Item::try_from("taste/reviewer".to_owned()).unwrap()));
        assert!(items.contains_key(&crate::account_copy::Item::try_from("taste/worker-profiles".to_owned()).unwrap()));
        assert_eq!(Tastes::from_copy_items(&items).unwrap(), tastes);
        let broken = crate::account_copy::Version { value: json!("nope"), secrets: BTreeMap::new() };
        assert_eq!(Tastes::normalize_copy_item(&captains, &broken).value, json!("nope"));
    }

    fn site() -> TasteScope {
        TasteScope::Project { repo: SITE.into() }
    }

    fn preset(profile: Profile) -> Taste {
        profile.preset().unwrap()
    }

    #[test]
    fn each_preset_profiles_as_itself_and_applying_one_keeps_skills_and_context() {
        for profile in Profile::PRESETS {
            assert_eq!(preset(profile).profile(), profile);
        }
        assert_eq!(Taste::default().profile(), Profile::Balanced);
        assert_eq!(Profile::Custom.preset(), None);

        let skills = vec![SkillRef { name: "tdd".into(), enabled: true }];
        let context = vec!["Deploys go through the VPS.".to_owned()];
        let mut taste = Taste { skills: skills.clone(), context: context.clone(), ..Taste::default() };
        taste.apply(Profile::Aggressive);
        assert_eq!(taste.profile(), Profile::Aggressive);
        assert_eq!((taste.parallel, taste.autonomy, &taste.skills, &taste.context), (8, Autonomy::ReportAtEnd, &skills, &context));
        let before = taste.clone();
        taste.apply(Profile::Custom);
        assert_eq!(taste, before, "Custom changed a knob");
    }

    #[test]
    fn one_changed_knob_makes_a_preset_custom_but_skills_and_context_do_not() {
        let mut taste = preset(Profile::Economical);
        taste.context.push("Ship on Fridays.".into());
        assert_eq!(taste.profile(), Profile::Economical);
        taste.parallel = 4;
        assert_eq!(taste.profile(), Profile::Custom);
    }

    #[test]
    fn a_project_resolves_as_the_captains_with_its_patch_on_top() {
        let patch = TastePatch { parallel: Some(4), autonomy: Some(Autonomy::ReportAtEnd), ..TastePatch::default() };
        let tastes = Tastes { captains: preset(Profile::Economical), projects: BTreeMap::from([(SITE.to_owned(), patch)]), ..Tastes::default() };
        let expected = Taste {
            strategy: Strategy::CheapWithGuidance,
            ladder: vec![ModelRef::named("deepseek-v4.1-flash"), ModelRef::named("glm-5.3"), ModelRef::named("claude-opus-5-5")],
            hard_model: Some(ModelRef::named("claude-opus-5-5")),
            parallel: 4,
            autonomy: Autonomy::ReportAtEnd,
            review_diffs: true,
            second_opinion: false,
            skills: vec![],
            context: vec![],
            tier: Tier::High,
            default_model: None,
            default_effort: None,
            max_effort: Effort::Max,
            budget_cents: 0,
            time_limit_min: 480,
            box_budget_cents: 0,
            box_size: None,
            escalate_breaking_change: true,
            escalate_process_failure: true,
            escalate_over_lines: 500,
            escalate_areas: vec!["Release and version bumps".into(), "CI and deploy config".into(), "Sign-in, tokens and secrets".into()],
            manage_only: true,
            claude_memory: true,
            worker_claude_memory: true,
        };
        assert_eq!(tastes.resolve(&site()), expected);
        assert_eq!(tastes.resolve(&TasteScope::Project { repo: "github.com/acme/api".into() }).parallel, 2, "a project with no patch is the Capitães'");
        assert_eq!(tastes.resolve(&TasteScope::Orchestrator).parallel, 5);
    }

    #[test]
    fn a_project_field_set_to_the_inherited_value_is_dropped_and_an_empty_patch_goes() {
        let mut tastes = Tastes::default();
        tastes.set(&site(), TastePatch { parallel: Some(4), autonomy: Some(Autonomy::DecideAndTell), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 4 } }));
        tastes.set(&site(), TastePatch { second_opinion: Some(true), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 4, "second_opinion": true } }), "a later set merges");
        tastes.set(&site(), TastePatch { parallel: Some(5), second_opinion: Some(false), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({}));
    }

    #[test]
    fn a_set_that_breaks_a_taste_changes_nothing() {
        let mut tastes = Tastes::default();
        tastes.set(&TasteScope::Captains, TastePatch { autonomy: Some(Autonomy::AskFirst), ..TastePatch::default() }).unwrap();
        assert_eq!(tastes.captains.autonomy, Autonomy::AskFirst);
        let before = tastes.clone();
        let wide = TastePatch { parallel: Some(101), autonomy: Some(Autonomy::ReportAtEnd), ..TastePatch::default() };
        assert_eq!(tastes.set(&TasteScope::Captains, wide.clone()), Err("parallel workers go from 1 to 100".into()));
        assert_eq!(tastes.set(&site(), wide), Err("parallel workers go from 1 to 100".into()));
        let effort = TastePatch { default_effort: Some(Effort::Max), max_effort: Some(Effort::High), ..TastePatch::default() };
        assert_eq!(tastes.set(&site(), effort), Err("the default effort is above the most effort allowed".into()));
        assert_eq!(tastes.set(&TasteScope::Captains, TastePatch { time_limit_min: Some(0), ..TastePatch::default() }), Err("the time limit goes from 1 to 10080 minutes".into()));
        assert_eq!(tastes.set(&TasteScope::Orchestrator, TastePatch { ladder: Some(vec![]), ..TastePatch::default() }), Err("the model ladder needs at least one model".into()));
        assert_eq!(tastes.set(&TasteScope::Project { repo: " ".into() }, TastePatch::default()), Err("a project's taste needs the project's repo".into()));
        assert_eq!(tastes, before);
    }

    #[test]
    fn a_reset_makes_a_project_inherit_and_gives_the_captains_the_default() {
        let mut tastes = Tastes::default();
        tastes.set(&site(), TastePatch { parallel: Some(4), review_diffs: Some(false), ..TastePatch::default() }).unwrap();
        tastes.reset(&site(), TasteField::Parallel);
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "review_diffs": false } }));
        tastes.reset(&site(), TasteField::ReviewDiffs);
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({}));

        tastes.set(&TasteScope::Captains, TastePatch { hard_model: Some(ModelRef { provider: Some("cursor".into()), model: "grok-4.7".into() }), parallel: Some(1), ..TastePatch::default() }).unwrap();
        tastes.reset(&TasteScope::Captains, TasteField::HardModel);
        assert_eq!((tastes.captains.hard_model, tastes.captains.parallel), (Some(ModelRef::named("claude-opus-5-5")), 1));
    }

    #[test]
    fn each_presets_brief() {
        assert_eq!(
            preset(Profile::Economical).brief(),
            "Run up to 2 workers at once.
Start each worker on deepseek-v4.1-flash with a detailed brief that names the files, the contracts and a command that proves the result, and re-brief it narrower before moving it up to glm-5.3.
Send hard work, such as architecture calls or a bug whose cause is unknown, straight to claude-opus-5-5.
Ask the user before you open a pull request or delete data.
Review each worker's diff before you accept its work."
        );
        assert_eq!(
            preset(Profile::Balanced).brief(),
            "Run up to 5 workers at once.
Start each worker on glm-5.3, and when it fails twice move it one rung up the ladder: claude-sonnet-5-5, then claude-opus-5-5.
Send hard work, such as architecture calls or a bug whose cause is unknown, straight to claude-opus-5-5.
Decide on your own and tell the user what you decided.
Review each worker's diff before you accept its work."
        );
        assert_eq!(
            preset(Profile::Aggressive).brief(),
            "Run up to 8 workers at once.
Start each worker on claude-opus-5-5 directly, without trying a cheaper model first.
Send hard work, such as architecture calls or a bug whose cause is unknown, straight to claude-opus-5-5.
Tell the user only at the end, with the result.
Have another model review a risky change before it goes ahead."
        );
    }

    #[test]
    fn a_brief_names_enabled_skills_and_context_notes_and_a_one_rung_ladder() {
        let taste = Taste {
            ladder: vec![ModelRef { provider: Some("opencode".into()), model: "glm-5.3".into() }],
            hard_model: None,
            parallel: 1,
            skills: vec![SkillRef { name: "tdd".into(), enabled: true }, SkillRef { name: "deslop".into(), enabled: false }],
            context: vec!["  Deploys go through the VPS. ".into(), " ".into()],
            ..preset(Profile::Economical)
        };
        assert_eq!(
            taste.brief(),
            "Run one worker at a time.
Start each worker on glm-5.3 through opencode with a detailed brief that names the files, the contracts and a command that proves the result, and re-brief it narrower when it fails.
Send hard work, such as architecture calls or a bug whose cause is unknown, straight to glm-5.3 through opencode.
Ask the user before you open a pull request or delete data.
Review each worker's diff before you accept its work.
Follow the tdd skill.
Deploys go through the VPS."
        );
        let escalating = Taste { strategy: Strategy::EscalateOnFailure, ..taste.clone() };
        assert_eq!(escalating.brief().lines().nth(1), Some("Start each worker on glm-5.3 through opencode, and when it fails twice ask the user."));
        let empty = Taste { ladder: vec![], ..taste };
        assert_eq!(empty.brief().lines().take(2).collect::<Vec<_>>(), ["Run one worker at a time.", "Ask the user before you open a pull request or delete data."], "a ladder with no model has no strategy");
    }

    #[test]
    fn tastes_travel_as_json_with_a_project_naming_only_its_own_fields() {
        let patch = TastePatch { parallel: Some(4), hard_model: Some(ModelRef { provider: Some("cursor".into()), model: "grok-4.7".into() }), ..TastePatch::default() };
        let tastes = Tastes { orchestrator: preset(Profile::Aggressive), projects: BTreeMap::from([(SITE.to_owned(), patch)]), ..Tastes::default() };
        let opus = json!({ "model": "claude-opus-5-5" });
        let areas = json!(["Release and version bumps", "CI and deploy config", "Sign-in, tokens and secrets"]);
        let json = json!({
            "orchestrator": {
                "strategy": "strongest_first", "ladder": [opus], "hard_model": opus, "parallel": 8, "autonomy": "report_at_end",
                "review_diffs": false, "second_opinion": true, "skills": [], "context": [],
                "tier": "high", "default_model": null, "default_effort": null, "max_effort": "max", "budget_cents": 0, "time_limit_min": 480, "box_budget_cents": 0,
                "box_size": null, "escalate_breaking_change": true, "escalate_process_failure": true, "escalate_over_lines": 500, "escalate_areas": areas,
                "manage_only": true, "claude_memory": true, "worker_claude_memory": true,
            },
            "captains": {
                "strategy": "escalate_on_failure", "ladder": [{ "model": "glm-5.3" }, { "model": "claude-sonnet-5-5" }, opus], "hard_model": opus,
                "parallel": 5, "autonomy": "decide_and_tell", "review_diffs": true, "second_opinion": false, "skills": [], "context": [],
                "tier": "high", "default_model": null, "default_effort": null, "max_effort": "max", "budget_cents": 0, "time_limit_min": 480, "box_budget_cents": 0,
                "box_size": null, "escalate_breaking_change": true, "escalate_process_failure": true, "escalate_over_lines": 500, "escalate_areas": areas,
                "manage_only": true, "claude_memory": true, "worker_claude_memory": true,
            },
            "projects": { "github.com/acme/site": { "hard_model": { "provider": "cursor", "model": "grok-4.7" }, "parallel": 4 } },
            "placement": { "prefer": [], "avoid": [], "heavy": "worktree" },
            "reviewer": { "model": null, "rounds": 2, "rules": [], "projects": {} },
            "worker_profiles": { "account": [], "projects": {}, "proposed": [] },
            "boxes_at_once": 8,
            "box_cap": 8,
        });
        assert_eq!(serde_json::to_value(&tastes).unwrap(), json);
        let mut before_the_team = json.clone();
        before_the_team.as_object_mut().unwrap().retain(|field, _| !USER_ONLY.contains(&field.as_str()));
        assert_eq!(serde_json::from_value::<Tastes>(before_the_team).unwrap(), tastes, "a taste.json from before the team reads with the defaults");
        assert_eq!(serde_json::from_value::<Tastes>(json).unwrap(), tastes);
        assert_eq!(serde_json::from_value::<Tastes>(json!({})).unwrap(), Tastes::default());
        let no_hard_model = Taste { hard_model: None, ..Taste::default() };
        assert_eq!(serde_json::from_value::<Taste>(serde_json::to_value(&no_hard_model).unwrap()).unwrap(), no_hard_model, "a null hard model is not the default one");
    }

    #[test]
    fn check_refuses_an_empty_ladder_a_nameless_model_and_more_than_a_hundred_workers() {
        assert_eq!(Taste::default().check(), Ok(()));
        assert_eq!(Taste { ladder: vec![], ..Taste::default() }.check(), Err("the model ladder needs at least one model".into()));
        assert_eq!(Taste { hard_model: Some(ModelRef::named(" ")), ..Taste::default() }.check(), Err("every model needs a name".into()));
        assert_eq!(Taste { parallel: 101, ..Taste::default() }.check(), Err("parallel workers go from 1 to 100".into()));
        assert_eq!(Taste { parallel: 0, ..Taste::default() }.check(), Err("parallel workers go from 1 to 100".into()));
        let project = TastePatch { ladder: Some(vec![]), ..TastePatch::default() };
        let tastes = Tastes { projects: BTreeMap::from([(SITE.to_owned(), project)]), ..Tastes::default() };
        assert_eq!(tastes.check(), Err("the model ladder needs at least one model".into()), "a project is checked resolved");
    }

    /// The page previews the brief and applies presets in JavaScript; this file holds both sides to the same text.
    #[test]
    fn the_ui_fixture_holds_the_presets_and_the_briefs() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../ui/taste.fixture.json")).unwrap();
        for (name, profile) in [("economical", Profile::Economical), ("balanced", Profile::Balanced), ("aggressive", Profile::Aggressive)] {
            assert_eq!(serde_json::from_value::<Taste>(fixture["presets"][name].clone()).unwrap(), preset(profile), "the fixture's {name} preset");
        }
        let cases = fixture["briefs"].as_array().unwrap();
        assert_eq!(cases.len(), 8);
        for case in cases {
            let taste: Taste = serde_json::from_value(case["taste"].clone()).unwrap();
            assert_eq!(taste.brief(), case["brief"].as_str().unwrap());
        }
    }

    #[test]
    fn the_old_cost_style_seeds_the_captains_once_and_a_saved_taste_wins() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(Some(dir.path())), None);
        std::fs::write(dir.path().join("orchestration.json"), r#"{"style":"quality","max_workers":3}"#).unwrap();
        let seeded = load(Some(dir.path())).unwrap();
        assert_eq!((seeded.captains.profile(), seeded.orchestrator.profile(), seeded.projects.len()), (Profile::Aggressive, Profile::Balanced, 0));
        let written: Tastes = serde_json::from_slice(&std::fs::read(dir.path().join("taste.json")).unwrap()).unwrap();
        assert_eq!(written, seeded);

        let saved = Tastes { captains: preset(Profile::Economical), ..Tastes::default() };
        save(&saved, Some(dir.path())).unwrap();
        assert_eq!(load(Some(dir.path())), Some(saved), "taste.json wins over the style");
        assert_eq!(load(None), None);

        std::fs::write(dir.path().join("taste.json"), "{ half a fi").unwrap();
        assert_eq!(load(Some(dir.path())), None);
        assert_eq!(std::fs::read_to_string(dir.path().join("taste.json")).unwrap(), "{ half a fi", "the style did not seed over a taste.json it couldn't read");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2, "no partial file is left behind");
    }

    #[test]
    fn a_taste_stored_on_the_old_parallel_defaults_loads_as_its_preset_again() {
        let dir = tempfile::tempdir().unwrap();
        let stored = Tastes {
            orchestrator: Taste { parallel: 6, ..preset(Profile::Aggressive) },
            captains: Taste { parallel: 3, ..preset(Profile::Balanced) },
            ..Tastes::default()
        };
        save(&stored, Some(dir.path())).unwrap();
        let loaded = load(Some(dir.path())).unwrap();
        assert_eq!((loaded.orchestrator.profile(), loaded.orchestrator.parallel), (Profile::Aggressive, 8));
        assert_eq!((loaded.captains.profile(), loaded.captains.parallel), (Profile::Balanced, 5));
        let written: Tastes = serde_json::from_slice(&std::fs::read(dir.path().join("taste.json")).unwrap()).unwrap();
        assert_eq!(written, stored, "the upgrade reads, it does not rewrite");

        let chosen = Taste { parallel: 4, ..preset(Profile::Balanced) };
        save(&Tastes { captains: chosen.clone(), ..Tastes::default() }, Some(dir.path())).unwrap();
        let loaded = load(Some(dir.path())).unwrap();
        assert_eq!((loaded.captains.profile(), loaded.captains.parallel), (Profile::Custom, 4), "a value the account really chose stays");
    }

    #[test]
    fn a_project_patch_that_set_only_the_old_default_inherits_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let stored = Tastes {
            captains: Taste { parallel: 3, ..preset(Profile::Balanced) },
            projects: BTreeMap::from([(SITE.to_owned(), TastePatch { parallel: Some(3), ..TastePatch::default() })]),
            ..Tastes::default()
        };
        save(&stored, Some(dir.path())).unwrap();
        let loaded = load(Some(dir.path())).unwrap();
        assert_eq!(serde_json::to_value(&loaded.projects).unwrap(), json!({}), "the patch was the old default, so the project inherits again");
        assert_eq!(loaded.resolve(&site()).parallel, 5);

        let with_more = Tastes {
            captains: Taste { parallel: 3, ..preset(Profile::Balanced) },
            projects: BTreeMap::from([(SITE.to_owned(), TastePatch { parallel: Some(3), review_diffs: Some(false), ..TastePatch::default() })]),
            ..Tastes::default()
        };
        save(&with_more, Some(dir.path())).unwrap();
        let loaded = load(Some(dir.path())).unwrap();
        assert_eq!(loaded.resolve(&site()).parallel, 3, "a patch that sets more than the old default is a choice and stays");

        let over_a_choice = Tastes {
            captains: Taste { parallel: 4, ..preset(Profile::Balanced) },
            projects: BTreeMap::from([(SITE.to_owned(), TastePatch { parallel: Some(3), ..TastePatch::default() })]),
            ..Tastes::default()
        };
        save(&over_a_choice, Some(dir.path())).unwrap();
        let loaded = load(Some(dir.path())).unwrap();
        assert_eq!(
            (loaded.captains.profile(), loaded.resolve(&site()).parallel),
            (Profile::Custom, 3),
            "a patch over a base that is no preset was chosen, so it stays"
        );
    }

    #[test]
    fn a_change_to_the_file_lands_and_a_refused_one_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let changed = change(Some(dir.path()), |tastes| tastes.set(&site(), TastePatch { parallel: Some(4), ..TastePatch::default() })).unwrap();
        let written: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("taste.json")).unwrap()).unwrap();
        assert_eq!(written["projects"], json!({ "github.com/acme/site": { "parallel": 4 } }));
        assert_eq!(serde_json::from_value::<Tastes>(written).unwrap(), changed);
        let refused = change(Some(dir.path()), |tastes| tastes.set(&TasteScope::Captains, TastePatch { parallel: Some(0), ..TastePatch::default() }));
        assert_eq!(refused, Err("parallel workers go from 1 to 100".into()));
        assert_eq!(load(Some(dir.path())), Some(changed));
    }

    #[test]
    fn a_projects_limits_beat_the_captains_and_a_preset_leaves_them_alone() {
        let mut tastes = Tastes::default();
        tastes.set(&TasteScope::Captains, TastePatch { tier: Some(Tier::Mid), budget_cents: Some(500), ..TastePatch::default() }).unwrap();
        tastes.set(&site(), TastePatch { tier: Some(Tier::Low), time_limit_min: Some(60), default_effort: Some(Effort::Medium), ..TastePatch::default() }).unwrap();
        assert_eq!(
            tastes.resolve(&site()).limits(),
            Limits { parallel: 5, tier: Tier::Low, default_effort: Some(Effort::Medium), max_effort: Effort::Max, budget_cents: 500, time_limit_min: 60, box_budget_cents: 0 }
        );
        assert_eq!(tastes.resolve(&TasteScope::Project { repo: "github.com/acme/api".into() }).limits().tier, Tier::Mid, "a project with no patch has the Captains' limits");
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "tier": "low", "default_effort": "medium", "time_limit_min": 60 } }));
        tastes.reset(&site(), TasteField::Tier);
        assert_eq!(tastes.resolve(&site()).limits().tier, Tier::Mid, "a reset project inherits the tier again");

        let mut captains = tastes.captains.clone();
        assert_eq!(captains.profile(), Profile::Balanced, "limits other than workers at once make no preset Custom");
        captains.apply(Profile::Aggressive);
        assert_eq!((captains.parallel, captains.tier, captains.budget_cents), (8, Tier::Mid, 500), "a preset sets workers at once and keeps the other limits");
    }

    #[test]
    fn a_narrowed_limit_is_the_tighter_of_the_two_and_no_cap_is_the_loosest() {
        let project = Limits { parallel: 5, tier: Tier::High, default_effort: None, max_effort: Effort::Max, budget_cents: 0, time_limit_min: 480, box_budget_cents: 300 };
        let parent = Limits { parallel: 2, tier: Tier::Mid, default_effort: Some(Effort::Xhigh), max_effort: Effort::High, budget_cents: 200, time_limit_min: 90, box_budget_cents: 0 };
        let narrowed = Limits { parallel: 2, tier: Tier::Mid, default_effort: Some(Effort::High), max_effort: Effort::High, budget_cents: 200, time_limit_min: 90, box_budget_cents: 300 };
        assert_eq!(project.narrow(parent), narrowed);
        assert_eq!(parent.narrow(project), narrowed);
    }

    #[test]
    fn admit_checks_the_tier_and_the_effort_and_fills_the_default() {
        let known = crate::claude_models::known();
        let cursor = crate::accounts::Account::builtin(crate::accounts::Provider::Cursor);
        let limits = Limits { tier: Tier::Mid, default_effort: Some(Effort::Medium), max_effort: Effort::High, ..Limits::default() };
        assert_eq!(limits.admit(&cursor, Some("glm-5.3"), None, &known), Ok(Some("medium".into())));
        assert_eq!(limits.admit(&cursor, Some("deepseek-v4.1-flash"), Some("high"), &known), Ok(Some("high".into())));
        assert_eq!(limits.admit(&cursor, Some("composer-2.5"), Some("fast"), &known), Ok(Some("fast".into())), "a level outside the five passes");
        assert_eq!(limits.admit(&cursor, Some("kimi-k3.1-pro"), None, &known), Err("kimi-k3.1-pro is a high-tier model; this project allows up to mid. Pick a mid model or raise Tier in Settings › Way of working".into()));
        assert_eq!(limits.admit(&cursor, Some("glm-5.3"), Some("xhigh"), &known), Err("effort xhigh is above high, the most this project allows. Ask for high or less, or raise Max effort in Settings › Way of working".into()));
        assert_eq!(Limits::default().admit(&cursor, None, None, &known), Ok(None), "with no tier cap a call may leave the model and the effort to the account");
    }

    #[test]
    fn the_tier_cap_sees_a_model_claudes_menu_added_at_its_familys_tier() {
        use crate::claude_models::{known, merged, MenuRow};
        let menu = [MenuRow { menu: "Fable".into(), version: "5.3".into() }, MenuRow { menu: "Haiku".into(), version: "5.5".into() }];
        let learned = merged(known(), &menu);
        let claude = crate::accounts::Account::builtin(crate::accounts::Provider::Claude);
        let low = Limits { tier: Tier::Low, ..Limits::default() };
        assert_eq!(low.admit(&claude, Some("claude-fable-5-3"), None, &learned), Err("claude-fable-5-3 is a mid-tier model; this project allows up to low. Pick a low model or raise Tier in Settings › Way of working".into()));
        assert_eq!(Limits { tier: Tier::Mid, ..Limits::default() }.admit(&claude, Some("claude-fable-5-3"), None, &learned), Ok(None));

        let mut cheap_fable = known();
        cheap_fable.iter_mut().filter(|m| m.slug.starts_with("claude-fable-")).for_each(|m| m.cost = crate::delegate::Cost::Low);
        let learned = merged(cheap_fable, &menu);
        assert_eq!(low.admit(&claude, Some("claude-fable-5-3"), None, &learned), Ok(None), "the family's tier, not the words of the id, which say mid");
    }

    /// A Claude login's `admit`, over the list pinned knows.
    fn admit_on_claude(limits: &Limits, model: Option<&str>) -> Result<Option<String>, String> {
        limits.admit(&crate::accounts::Account::builtin(crate::accounts::Provider::Claude), model, None, &crate::claude_models::known())
    }

    fn refused_as_unlisted(result: Result<Option<String>, String>, model: &str) {
        let error = result.expect_err("a model id the machine does not list passed the ceiling");
        assert!(error.starts_with(&format!("{model} is not in this machine's list of Claude models")), "{error}");
    }

    #[test]
    fn a_claude_model_id_the_machine_does_not_list_is_refused_under_a_mid_ceiling() {
        let mid = Limits { tier: Tier::Mid, ..Limits::default() };
        refused_as_unlisted(admit_on_claude(&mid, Some("claude-nova-9-9")), "claude-nova-9-9");
        refused_as_unlisted(admit_on_claude(&mid, Some("sonnet")), "sonnet");
    }

    #[test]
    fn a_claude_model_id_the_machine_does_not_list_is_refused_under_a_low_ceiling() {
        let low = Limits { tier: Tier::Low, ..Limits::default() };
        // Its words say Low, which is all the guess had.
        refused_as_unlisted(admit_on_claude(&low, Some("claude-lite-9-9")), "claude-lite-9-9");
    }

    #[test]
    fn a_listed_claude_slug_is_held_to_its_listed_tier_under_a_ceiling() {
        let known = crate::claude_models::known();
        let slug_of = |cost| known.iter().find(|m| m.cost == cost).map(|m| m.slug.clone()).expect("a listed model of each tier");
        let (low, mid, high) = (slug_of(crate::delegate::Cost::Low), slug_of(crate::delegate::Cost::Mid), slug_of(crate::delegate::Cost::High));
        let at_mid = Limits { tier: Tier::Mid, ..Limits::default() };
        assert_eq!(admit_on_claude(&at_mid, Some(&low)), Ok(None));
        assert_eq!(admit_on_claude(&at_mid, Some(&mid)), Ok(None));
        let refused = admit_on_claude(&at_mid, Some(&high)).unwrap_err();
        assert!(refused.contains("high-tier model") && refused.contains("allows up to mid"), "{refused}");
        let at_low = Limits { tier: Tier::Low, ..Limits::default() };
        assert_eq!(admit_on_claude(&at_low, Some(&low)), Ok(None));
        assert!(admit_on_claude(&at_low, Some(&mid)).unwrap_err().contains("mid-tier model"));
    }

    #[test]
    fn with_no_ceiling_a_claude_model_id_the_machine_does_not_list_passes_as_before() {
        let open = Limits::default();
        assert_eq!(admit_on_claude(&open, Some("claude-nova-9-9")), Ok(None));
        assert_eq!(admit_on_claude(&open, Some("sonnet")), Ok(None));
        assert_eq!(admit_on_claude(&open, None), Ok(None));
        let mid = Limits { tier: Tier::Mid, ..Limits::default() };
        let error = admit_on_claude(&mid, Some("")).unwrap_err();
        assert!(error.starts_with("name a model:"), "an empty model is no model under a tier cap: {error}");
    }

    #[test]
    fn an_account_not_held_here_still_has_its_claude_id_checked_against_the_list_under_a_ceiling() {
        let known = crate::claude_models::known();
        let mid = Limits { tier: Tier::Mid, ..Limits::default() };
        refused_as_unlisted(mid.admit_unheld(Some("claude-nova-9-9"), None, &known), "claude-nova-9-9");
        // Every other id is priced by its words, as before.
        for other in ["kimi-k3", "glm-5.3", "sonnet"] {
            assert_eq!(mid.admit_unheld(Some(other), None, &known), Ok(None), "{other}");
        }
        assert!(mid.admit_unheld(Some("claude-opus-5-5"), None, &known).unwrap_err().contains("high-tier"));
        // With no tier cap nothing is checked.
        assert_eq!(Limits::default().admit_unheld(Some("claude-nova-9-9"), None, &known), Ok(None));
    }

    #[test]
    fn a_ceiling_leaves_the_other_drivers_to_their_own_lists() {
        use crate::accounts::{Account, Auth, Provider};
        let mid = Limits { tier: Tier::Mid, ..Limits::default() };
        let known = crate::claude_models::known();
        let go = Account { id: "go".into(), name: "Go".into(), auth: Auth::OpenCodeGo, ..Account::builtin(Provider::Claude) };
        for account in [Account::builtin(Provider::Cursor), Account::builtin(Provider::Codex), go] {
            assert_eq!(mid.admit(&account, Some("claude-nova-9-9"), None, &known), Ok(None), "{}", account.id);
        }
    }

    #[test]
    fn the_box_budget_is_the_default_and_the_ceiling_of_a_boxs_budget() {
        let limits = Limits { box_budget_cents: 250, ..Limits::default() };
        assert_eq!(limits.box_budget_usd(None), Ok(Some(2.5)));
        assert_eq!(limits.box_budget_usd(Some(1.0)), Ok(Some(1.0)));
        assert_eq!(limits.box_budget_usd(Some(3.0)), Err("a box may spend at most US$ 2.50; name less, or raise Box budget in Settings › Way of working".into()));
        assert_eq!(Limits::default().box_budget_usd(None), Ok(None), "no box budget sets none");
        assert_eq!(Limits::default().box_budget_usd(Some(40.0)), Ok(Some(40.0)));
    }

    #[test]
    fn a_session_is_told_each_limit_pinned_enforces() {
        assert_eq!(
            Limits::default().hint(),
            "pinned enforces these delegation limits on delegate_task: at most 5 workers at once, models up to the high tier, effort up to max, 480 minutes per worker. A call past them is refused, and a worker past its time or budget is stopped; if a limit blocks you, tell the user to raise it in Settings › Way of working rather than working around it. The tier covers the model each worker runs on, not the models it picks for its own subagents."
        );
        let tight = Limits { parallel: 1, tier: Tier::Low, default_effort: Some(Effort::Low), max_effort: Effort::Medium, budget_cents: 150, time_limit_min: 45, box_budget_cents: 1000 };
        assert_eq!(
            tight.hint(),
            "pinned enforces these delegation limits on delegate_task: one worker at a time, models up to the low tier, effort up to medium (low when a call names none), 45 minutes per worker, US$ 1.50 per worker, US$ 10.00 per box. A call past them is refused, and a worker past its time or budget is stopped; if a limit blocks you, tell the user to raise it in Settings › Way of working rather than working around it. The tier covers the model each worker runs on, not the models it picks for its own subagents."
        );
        assert_eq!(tight.over_budget(Some(1.49), None), None);
        assert_eq!(
            tight.over_budget(None, Some("glm-5.3")),
            Some("glm-5.3 has no price, so pinned can't hold it to its budget of US$ 1.50. Set one in Settings › Usage › Model prices, or set the budget to 0".into()),
            "a spend that can't be known stops the run instead of leaving it with no cap"
        );
        assert_eq!(tight.over_budget(None, None), Some("its model has no price, so pinned can't hold it to its budget of US$ 1.50. Set one in Settings › Usage › Model prices, or set the budget to 0".into()));
        assert_eq!(tight.over_budget(Some(1.5), None), Some("pinned stopped it after it spent US$ 1.50, its budget of US$ 1.50".into()));
        assert_eq!(Limits::default().over_budget(Some(1000.0), None), None, "no budget, no stop");
        assert_eq!(Limits::default().over_budget(None, None), None, "no budget, an unknown spend stops nothing");
    }

    #[test]
    fn the_old_worker_cap_lowers_each_scope_once_and_the_file_goes() {
        let dir = tempfile::tempdir().unwrap();
        let mut seeded = Tastes::default();
        seeded.set(&site(), TastePatch { parallel: Some(6), ..TastePatch::default() }).unwrap();
        seeded.set(&TasteScope::Project { repo: "github.com/acme/api".into() }, TastePatch { parallel: Some(1), ..TastePatch::default() }).unwrap();
        save(&seeded, Some(dir.path())).unwrap();
        std::fs::write(dir.path().join("orchestration.json"), r#"{"max_workers":2}"#).unwrap();
        migrate_worker_cap(Some(dir.path()));
        let moved = load(Some(dir.path())).unwrap();
        assert_eq!((moved.orchestrator.parallel, moved.captains.parallel), (2, 2));
        assert_eq!(serde_json::to_value(&moved.projects).unwrap(), json!({ "github.com/acme/api": { "parallel": 1 }, "github.com/acme/site": { "parallel": 2 } }));
        assert!(!dir.path().join("orchestration.json").exists(), "the old file is gone, so the cap moves once");

        let fresh = tempfile::tempdir().unwrap();
        std::fs::write(fresh.path().join("orchestration.json"), r#"{"style":"quality","max_workers":100}"#).unwrap();
        migrate_worker_cap(Some(fresh.path()));
        assert_eq!(load(Some(fresh.path())).map(|t| (t.captains.profile(), t.captains.parallel)), Some((Profile::Aggressive, 8)), "the default cap lowers nothing, and the style still seeds");
        assert!(!fresh.path().join("orchestration.json").exists());
    }

    #[test]
    fn the_placement_policy_is_set_and_reset_for_the_account_and_never_for_one_project() {
        use crate::placement::{HeavyRule, Placement};
        let policy = Placement { prefer: vec!["m-mini".into()], avoid: vec!["m-book".into()], heavy: HeavyRule::Worktree };
        let mut tastes = Tastes::default();
        tastes.set(&TasteScope::Captains, TastePatch { placement: Some(policy.clone()), parallel: Some(4), ..TastePatch::default() }).unwrap();
        assert_eq!((&tastes.placement, tastes.captains.parallel), (&policy, 4), "a patch with other knobs sets both");
        assert_eq!(
            tastes.set(&site(), TastePatch { placement: Some(Placement::default()), ..TastePatch::default() }),
            Err("placement belongs to the account: set it for Captains (all), not for one project".into())
        );
        let both = Placement { avoid: vec!["m-mini".into()], ..policy.clone() };
        assert_eq!(tastes.set(&TasteScope::Captains, TastePatch { placement: Some(both), ..TastePatch::default() }), Err("machine m-mini is in both Prefer and Avoid; keep it in one".into()));
        assert_eq!(tastes.placement, policy, "a refused change keeps the policy");
        let stored = serde_json::to_value(&tastes).unwrap();
        assert_eq!(stored["placement"], json!({ "prefer": ["m-mini"], "avoid": ["m-book"], "heavy": "worktree" }));
        let mut older = stored.clone();
        older.as_object_mut().unwrap().remove("placement");
        assert_eq!(serde_json::from_value::<Tastes>(older).unwrap().placement, Placement::default(), "tastes kept before placement read with none");
        tastes.reset(&TasteScope::Captains, TasteField::Placement);
        assert_eq!((tastes.placement, tastes.captains.parallel), (Placement::default(), 4), "the reset clears the policy only");
    }

    /// `(size, from)` of a box cloning `repo` whose call names `asked`.
    fn sized(tastes: &Tastes, repo: Option<&str>, asked: Option<BoxSize>) -> (BoxSize, BoxSizeSource) {
        let choice = tastes.box_size(repo, asked);
        (choice.size(), choice.from())
    }

    #[test]
    fn a_box_takes_the_calls_size_then_the_projects_then_the_accounts_then_small() {
        let mut tastes = Tastes::default();
        assert_eq!(sized(&tastes, Some(SITE), None), (BoxSize::Small, BoxSizeSource::BuiltIn));
        assert_eq!(sized(&tastes, None, None), (BoxSize::Small, BoxSizeSource::BuiltIn));

        tastes.set(&TasteScope::Captains, TastePatch { box_size: Some(BoxSize::Large), ..TastePatch::default() }).unwrap();
        assert_eq!(sized(&tastes, Some(SITE), None), (BoxSize::Large, BoxSizeSource::Account));
        assert_eq!(sized(&tastes, None, None), (BoxSize::Large, BoxSizeSource::Account), "a box of no project takes the account's");

        tastes.set(&site(), TastePatch { box_size: Some(BoxSize::Medium), ..TastePatch::default() }).unwrap();
        assert_eq!(sized(&tastes, Some(SITE), None), (BoxSize::Medium, BoxSizeSource::Project));
        for other_form in ["https://github.com/acme/site.git", "git@github.com:acme/site.git", "https://github.com/Acme/Site"] {
            assert_eq!(sized(&tastes, Some(other_form), None), (BoxSize::Medium, BoxSizeSource::Project), "{other_form}");
        }
        assert_eq!(sized(&tastes, Some("github.com/acme/api"), None), (BoxSize::Large, BoxSizeSource::Account), "another project keeps the account's");

        assert_eq!(sized(&tastes, Some(SITE), Some(BoxSize::Small)), (BoxSize::Small, BoxSizeSource::Call));
        assert_eq!(sized(&tastes, None, Some(BoxSize::Medium)), (BoxSize::Medium, BoxSizeSource::Call));
        assert_eq!(serde_json::to_value(tastes.box_size(Some(SITE), None)).unwrap(), json!({ "size": "medium", "from": "project" }));
    }

    #[test]
    fn a_project_with_no_box_size_inherits_the_accounts() {
        let mut tastes = Tastes::default();
        tastes.set(&TasteScope::Captains, TastePatch { box_size: Some(BoxSize::Medium), ..TastePatch::default() }).unwrap();
        tastes.set(&site(), TastePatch { parallel: Some(4), ..TastePatch::default() }).unwrap();
        assert_eq!(tastes.resolve(&site()).box_size, Some(BoxSize::Medium));
        assert_eq!(sized(&tastes, Some(SITE), None), (BoxSize::Medium, BoxSizeSource::Account));

        tastes.set(&site(), TastePatch { box_size: Some(BoxSize::Medium), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 4 } }), "the account's size set on the project is dropped, as any field");
    }

    #[test]
    fn the_orchestrator_takes_no_box_size() {
        let mut tastes = Tastes::default();
        let before = tastes.clone();
        assert_eq!(
            tastes.set(&TasteScope::Orchestrator, TastePatch { box_size: Some(BoxSize::Large), ..TastePatch::default() }),
            Err("the box size belongs to the projects: set the account's for Captains (all), or one project's".into())
        );
        assert_eq!(tastes, before);
        assert_eq!(sized(&tastes, Some(SITE), None), (BoxSize::Small, BoxSizeSource::BuiltIn));
    }

    #[test]
    fn a_box_size_reset_makes_a_project_inherit_and_gives_the_account_small_back() {
        let mut tastes = Tastes::default();
        tastes.set(&TasteScope::Captains, TastePatch { box_size: Some(BoxSize::Medium), ..TastePatch::default() }).unwrap();
        tastes.set(&site(), TastePatch { box_size: Some(BoxSize::Large), parallel: Some(4), ..TastePatch::default() }).unwrap();
        tastes.reset(&site(), TasteField::BoxSize);
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 4 } }));
        assert_eq!(sized(&tastes, Some(SITE), None), (BoxSize::Medium, BoxSizeSource::Account));

        tastes.reset(&TasteScope::Captains, TasteField::BoxSize);
        assert_eq!(tastes.captains.box_size, None);
        assert_eq!(sized(&tastes, Some(SITE), None), (BoxSize::Small, BoxSizeSource::BuiltIn));
    }

    #[test]
    fn tastes_kept_before_the_box_size_read_with_none_and_open_small() {
        let older = json!({
            "captains": { "parallel": 3, "box_budget_cents": 500 },
            "projects": { "github.com/acme/site": { "parallel": 2 } }
        });
        let tastes: Tastes = serde_json::from_value(older).unwrap();
        assert_eq!((tastes.captains.box_size, tastes.orchestrator.box_size, tastes.projects[SITE].box_size), (None, None, None));
        assert_eq!(sized(&tastes, Some(SITE), None), (BoxSize::Small, BoxSizeSource::BuiltIn));
        assert_eq!(serde_json::to_value(&tastes.captains).unwrap()["box_size"], Value::Null, "the account's is written, null included");
        assert!(serde_json::to_value(&tastes.projects[SITE]).unwrap().get("box_size").is_none(), "a project's patch leaves it out");
    }

    const ZERO_NOTE: &str = "Boxes at once is 0: agents cannot start an Upstash Box on this account. Never pass target.box to delegate_task or box to send_to_captain, and never call open_box; run the work on the connected machines that orchestrator_capabilities lists under machines.";

    #[test]
    fn boxes_at_once_reads_8_without_the_field_and_takes_the_hubs_cap_above_it() {
        let old: Tastes = serde_json::from_value(json!({ "captains": { "parallel": 3 } })).unwrap();
        assert_eq!((old.boxes_at_once, old.box_cap, old.boxes_limit()), (8, 8, 8), "tastes from before the field");
        assert_eq!(Tastes::default().boxes_limit(), 8, "a fresh account");
        let patch: TastePatch = serde_json::from_value(json!({ "parallel": 2 })).unwrap();
        assert_eq!(patch.boxes_at_once, None, "an old app's patch leaves it alone");
        let stored: Tastes = serde_json::from_value(json!({ "boxes_at_once": 20, "box_cap": 8 })).unwrap();
        assert_eq!(stored.boxes_limit(), 8, "a stored 20 is the cap of 8");

        let mut tastes = Tastes::default();
        let set = |n: u32| TastePatch { boxes_at_once: Some(n), ..TastePatch::default() };
        assert_eq!(tastes.set(&TasteScope::Project { repo: SITE.into() }, set(2)), Err("Boxes at once belongs to the account: set it for Captains (all), not for one project".into()));
        tastes.set(&TasteScope::Captains, set(20)).unwrap();
        assert_eq!((tastes.boxes_at_once, tastes.boxes_limit()), (20, 8));
        tastes.set(&TasteScope::Captains, TastePatch { parallel: Some(2), boxes_at_once: Some(0), ..TastePatch::default() }).unwrap();
        assert_eq!((tastes.boxes_at_once, tastes.captains.parallel), (0, 2), "with another field in the same patch");
        tastes.reset(&TasteScope::Captains, TasteField::BoxesAtOnce);
        assert_eq!(tastes.boxes_at_once, 8, "a reset gives the default back");
    }

    #[test]
    fn boxes_at_once_is_the_users_own() {
        let refused = Err("boxes_at_once is changed only by the user, on the Way of working page; nothing changed".to_owned());
        let patch = TastePatch { parallel: Some(2), boxes_at_once: Some(3), ..TastePatch::default() };
        assert_eq!(refuse_review_settings(patch.review_settings()), refused);
        assert_eq!(refuse_review_settings(TasteField::BoxesAtOnce.review_setting()), refused);
    }

    #[test]
    fn boxes_at_once_tells_the_agents_the_number_or_to_use_the_machines() {
        let with = |n: u32| Tastes { boxes_at_once: n, ..Tastes::default() };
        assert_eq!(with(3).boxes_note(), "Boxes at once: 3. The account's agents may have 3 Upstash Boxes open at once; one more is refused.");
        assert_eq!(with(1).boxes_note(), "Boxes at once: 1. The account's agents may have 1 Upstash Box open at once; one more is refused.");
        assert_eq!(with(0).boxes_note(), ZERO_NOTE);
        assert_eq!(with(3).boxes_brief(), None, "the prompt says nothing more while boxes are allowed");
        assert_eq!(with(0).boxes_brief().as_deref(), Some(ZERO_NOTE));
    }

    fn areas(list: &[&str]) -> Vec<String> {
        list.iter().map(|area| (*area).to_owned()).collect()
    }

    #[test]
    fn the_escalation_settings_default_on_every_preset_and_read_so_from_older_tastes() {
        let defaults = (true, true, 500, areas(&["Release and version bumps", "CI and deploy config", "Sign-in, tokens and secrets"]));
        for taste in [Taste::default(), preset(Profile::Economical), preset(Profile::Balanced), preset(Profile::Aggressive)] {
            assert_eq!((taste.escalate_breaking_change, taste.escalate_process_failure, taste.escalate_over_lines, taste.escalate_areas), defaults.clone());
        }
        let old: Tastes = serde_json::from_value(json!({ "captains": { "parallel": 3 }, "projects": { "github.com/acme/site": { "parallel": 2 } } })).unwrap();
        assert_eq!((old.captains.escalate_over_lines, old.orchestrator.escalate_areas.len()), (500, 3), "tastes from before the fields");
        assert_eq!(old.resolve(&site()).escalate_over_lines, 500);
        let patch: TastePatch = serde_json::from_value(json!({ "parallel": 2 })).unwrap();
        assert_eq!((patch.escalate_breaking_change, patch.escalate_process_failure, patch.escalate_over_lines, patch.escalate_areas), (None, None, None, None), "an old app's patch leaves them alone");
    }

    #[test]
    fn a_project_sets_inherits_and_resets_each_escalation_setting() {
        let mut tastes = Tastes::default();
        let patch = TastePatch {
            escalate_breaking_change: Some(false),
            escalate_process_failure: Some(false),
            escalate_over_lines: Some(750),
            escalate_areas: Some(areas(&["Billing"])),
            ..TastePatch::default()
        };
        tastes.set(&site(), patch).unwrap();
        assert_eq!(
            serde_json::to_value(&tastes.projects).unwrap(),
            json!({ "github.com/acme/site": { "escalate_breaking_change": false, "escalate_process_failure": false, "escalate_over_lines": 750, "escalate_areas": ["Billing"] } })
        );
        let resolved = tastes.resolve(&site());
        assert_eq!((resolved.escalate_breaking_change, resolved.escalate_over_lines, resolved.escalate_areas), (false, 750, areas(&["Billing"])));
        assert_eq!(tastes.resolve(&TasteScope::Project { repo: "github.com/acme/api".into() }).escalate_over_lines, 500, "another project inherits the Captains'");

        tastes.set(&site(), TastePatch { escalate_over_lines: Some(500), ..TastePatch::default() }).unwrap();
        assert!(tastes.projects[SITE].escalate_over_lines.is_none(), "the Captains' value set on the project is dropped");
        for field in [TasteField::EscalateBreakingChange, TasteField::EscalateProcessFailure, TasteField::EscalateAreas] {
            tastes.reset(&site(), field);
        }
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({}), "each reset inherits again");

        tastes.set(&TasteScope::Captains, TastePatch { escalate_over_lines: Some(80), escalate_areas: Some(vec![]), escalate_process_failure: Some(false), ..TastePatch::default() }).unwrap();
        assert_eq!((tastes.resolve(&site()).escalate_over_lines, tastes.resolve(&site()).escalate_areas.len()), (80, 0), "a project inherits the Captains' change");
        tastes.reset(&TasteScope::Captains, TasteField::EscalateOverLines);
        tastes.reset(&TasteScope::Captains, TasteField::EscalateAreas);
        assert_eq!((tastes.captains.escalate_over_lines, tastes.captains.escalate_areas.len(), tastes.captains.escalate_process_failure), (500, 3, false), "a reset gives the default back");
        tastes.reset(&TasteScope::Captains, TasteField::EscalateProcessFailure);
        assert!(tastes.captains.escalate_process_failure);
    }

    #[test]
    fn the_line_limit_goes_from_1_to_100000_and_a_refused_one_changes_nothing() {
        let mut tastes = Tastes::default();
        let before = tastes.clone();
        let lines = |n: u32| TastePatch { escalate_over_lines: Some(n), parallel: Some(2), ..TastePatch::default() };
        for n in [0, 100_001] {
            for scope in [TasteScope::Orchestrator, TasteScope::Captains, site()] {
                assert_eq!(tastes.set(&scope, lines(n)), Err("the line limit goes from 1 to 100000 changed lines".into()), "{n} on {}", scope.name());
            }
        }
        assert_eq!(tastes, before);
        tastes.set(&TasteScope::Captains, lines(1)).unwrap();
        tastes.set(&site(), lines(100_000)).unwrap();
        assert_eq!((tastes.captains.escalate_over_lines, tastes.resolve(&site()).escalate_over_lines), (1, 100_000));
    }

    #[test]
    fn blank_escalation_areas_are_dropped_and_an_empty_list_is_allowed() {
        let mut tastes = Tastes::default();
        tastes.set(&TasteScope::Captains, TastePatch { escalate_areas: Some(areas(&["  Billing ", "", "   ", "Auth"])), ..TastePatch::default() }).unwrap();
        assert_eq!(tastes.captains.escalate_areas, areas(&["Billing", "Auth"]));
        tastes.set(&site(), TastePatch { escalate_areas: Some(areas(&[" "])), ..TastePatch::default() }).unwrap();
        assert_eq!(tastes.resolve(&site()).escalate_areas, Vec::<String>::new());
        assert_eq!(tastes.projects[SITE].escalate_areas, Some(vec![]), "an emptied list is the project's own");
    }

    #[test]
    fn the_escalation_settings_leave_the_profile_and_the_brief_alone() {
        let mut taste = preset(Profile::Economical);
        let brief = taste.brief();
        taste.escalate_breaking_change = false;
        taste.escalate_process_failure = false;
        taste.escalate_over_lines = 42;
        taste.escalate_areas = areas(&["Billing"]);
        assert_eq!((taste.profile(), taste.brief()), (Profile::Economical, brief));
        taste.apply(Profile::Aggressive);
        assert_eq!((taste.profile(), taste.escalate_breaking_change, taste.escalate_over_lines, taste.escalate_areas.clone()), (Profile::Aggressive, false, 42, areas(&["Billing"])), "a preset keeps them");
    }

    #[test]
    fn the_escalation_settings_are_the_users_own() {
        let names = ["escalate_breaking_change", "escalate_process_failure", "escalate_over_lines", "escalate_areas"];
        let refused = |name: &str| Err(format!("{name} is changed only by the user, on the Way of working page; nothing changed"));
        let patches = [
            TastePatch { escalate_breaking_change: Some(false), ..TastePatch::default() },
            TastePatch { escalate_process_failure: Some(false), ..TastePatch::default() },
            TastePatch { escalate_over_lines: Some(9), ..TastePatch::default() },
            TastePatch { escalate_areas: Some(vec![]), ..TastePatch::default() },
        ];
        let fields = [TasteField::EscalateBreakingChange, TasteField::EscalateProcessFailure, TasteField::EscalateOverLines, TasteField::EscalateAreas];
        for ((name, patch), field) in names.into_iter().zip(patches).zip(fields) {
            assert_eq!(refuse_review_settings(patch.review_settings()), refused(name));
            assert_eq!(refuse_review_settings(TastePatch { parallel: Some(2), ..patch }.review_settings()), refused(name), "mixed with an allowed field");
            assert_eq!(refuse_review_settings(field.review_setting()), refused(name));
        }
        let all = TastePatch { escalate_over_lines: Some(9), escalate_areas: Some(vec![]), ..TastePatch::default() };
        assert_eq!(refuse_review_settings(all.review_settings()), Err("escalate_over_lines and escalate_areas are changed only by the user, on the Way of working page; nothing changed".into()));
        assert_eq!(serde_json::to_value(TasteField::EscalateOverLines).unwrap(), json!("escalate_over_lines"));
    }

    #[test]
    fn the_offline_copy_carries_no_escalation_item_and_a_handback_keeps_them() {
        let mut stored = Tastes::default();
        let mine = TastePatch { escalate_breaking_change: Some(false), escalate_over_lines: Some(90), escalate_areas: Some(areas(&["Billing"])), ..TastePatch::default() };
        stored.set(&TasteScope::Captains, mine.clone()).unwrap();
        stored.set(&TasteScope::Orchestrator, mine).unwrap();
        stored.set(&TasteScope::Captains, TastePatch { boxes_at_once: Some(3), ..TastePatch::default() }).unwrap();
        stored.set(&site(), TastePatch { parallel: Some(4), ..TastePatch::default() }).unwrap();
        let items = stored.copy_items();
        assert!(!items.keys().any(|item| item.as_str().contains("escalate_")), "{:?}", items.keys().map(|item| item.as_str()).collect::<Vec<_>>());

        let mut edited = Tastes::default();
        edited.set(&TasteScope::Captains, TastePatch { parallel: Some(2), ..TastePatch::default() }).unwrap();
        let back = stored.with_copy_items(&edited.copy_items()).unwrap();
        assert_eq!(back.captains.parallel, 2, "the copy's edit lands");
        assert_eq!((back.captains.escalate_breaking_change, back.captains.escalate_over_lines, back.captains.escalate_areas.clone()), (false, 90, areas(&["Billing"])));
        assert_eq!((back.orchestrator.escalate_over_lines, back.boxes_at_once), (90, 3), "what the copy does not carry keeps its stored value");
        assert!(back.projects.is_empty(), "a project the copy does not name is gone");
    }

    #[test]
    fn manage_only_is_on_by_default_and_a_project_sets_inherits_and_resets_it() {
        assert!(Taste::default().manage_only);
        assert!(Tastes::default().resolve(&TasteScope::Project { repo: "github.com/acme/none".into() }).manage_only, "a project with no tastes is Manage only");
        assert!(!crate::agent::manage_only_prompt().contains("Step in and do the task yourself"), "the manage-only prompt has no step-in line");
        let old: Tastes = serde_json::from_value(json!({ "captains": { "parallel": 3 }, "projects": { "github.com/acme/site": { "parallel": 2 } } })).unwrap();
        assert_eq!((old.captains.manage_only, old.resolve(&site()).manage_only), (true, true), "tastes without the field take the default");
        let api = TasteScope::Project { repo: "github.com/acme/api".into() };

        let mut tastes = Tastes::default();
        tastes.set(&site(), TastePatch { manage_only: Some(false), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "manage_only": false } }), "an explicit off is kept");
        assert_eq!((tastes.resolve(&site()).manage_only, tastes.resolve(&api).manage_only), (false, true), "another project stays on");
        tastes.set(&site(), TastePatch { parallel: Some(2), manage_only: Some(true), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 2 } }), "the Captains' value set on the project is dropped");
        tastes.set(&site(), TastePatch { manage_only: Some(false), ..TastePatch::default() }).unwrap();
        tastes.reset(&site(), TasteField::ManageOnly);
        assert_eq!((tastes.projects[SITE].manage_only, tastes.resolve(&site()).manage_only), (None, true), "a reset inherits again");

        tastes.set(&TasteScope::Captains, TastePatch { manage_only: Some(false), ..TastePatch::default() }).unwrap();
        assert_eq!((tastes.resolve(&site()).manage_only, tastes.resolve(&api).manage_only), (false, false), "a project without its own inherits the Captains'");
        tastes.reset(&TasteScope::Captains, TasteField::ManageOnly);
        assert!(tastes.captains.manage_only, "a reset gives the Captains on back");
        assert_eq!(tastes.set(&TasteScope::Orchestrator, TastePatch { manage_only: Some(true), ..TastePatch::default() }), Ok(()), "the orchestrator's scope takes it and nothing reads it");
    }

    #[test]
    fn claude_memory_is_on_by_default_and_a_project_sets_inherits_and_resets_each_switch() {
        assert!(Taste::default().claude_memory && Taste::default().worker_claude_memory);
        let old: Tastes = serde_json::from_value(json!({ "captains": { "parallel": 3 }, "projects": { "github.com/acme/site": { "parallel": 2 } } })).unwrap();
        for scope in [&old.orchestrator, &old.captains, &old.resolve(&site())] {
            assert_eq!((scope.claude_memory, scope.worker_claude_memory), (true, true), "tastes from before the fields");
        }
        let old_orchestrator: Taste = serde_json::from_value(json!({ "parallel": 3 })).unwrap();
        assert_eq!((old_orchestrator.claude_memory, old_orchestrator.worker_claude_memory), (true, true), "a taste saved before the fields");
        let api = TasteScope::Project { repo: "github.com/acme/api".into() };

        let mut tastes = Tastes::default();
        tastes.set(&site(), TastePatch { claude_memory: Some(false), worker_claude_memory: Some(false), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "claude_memory": false, "worker_claude_memory": false } }));
        let resolved = |tastes: &Tastes, scope: &TasteScope| (tastes.resolve(scope).claude_memory, tastes.resolve(scope).worker_claude_memory);
        assert_eq!((resolved(&tastes, &site()), resolved(&tastes, &api)), ((false, false), (true, true)), "another project stays on");
        tastes.set(&site(), TastePatch { parallel: Some(2), claude_memory: Some(true), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 2, "worker_claude_memory": false } }), "the Captains' value set on the project is dropped");
        tastes.reset(&site(), TasteField::WorkerClaudeMemory);
        assert_eq!(resolved(&tastes, &site()), (true, true), "a reset inherits again");

        tastes.set(&TasteScope::Captains, TastePatch { worker_claude_memory: Some(false), ..TastePatch::default() }).unwrap();
        assert_eq!((resolved(&tastes, &site()), resolved(&tastes, &api)), ((true, false), (true, false)), "a project without its own inherits the Captains'");
        tastes.set(&site(), TastePatch { worker_claude_memory: Some(true), ..TastePatch::default() }).unwrap();
        assert_eq!((resolved(&tastes, &site()), resolved(&tastes, &api)), ((true, true), (true, false)), "a project's own beats the Captains'");
        tastes.reset(&TasteScope::Captains, TasteField::WorkerClaudeMemory);
        assert!(tastes.captains.worker_claude_memory, "a reset gives the Captains on back");

        tastes.set(&TasteScope::Orchestrator, TastePatch { claude_memory: Some(false), ..TastePatch::default() }).unwrap();
        assert_eq!((tastes.resolve(&TasteScope::Orchestrator).claude_memory, tastes.captains.claude_memory), (false, true), "the orchestrator's switch is its own");
        tastes.reset(&TasteScope::Orchestrator, TasteField::ClaudeMemory);
        assert!(tastes.orchestrator.claude_memory);
    }

    #[test]
    fn the_claude_memory_switches_leave_the_profile_and_the_brief_alone() {
        let mut taste = preset(Profile::Balanced);
        let brief = taste.brief();
        taste.claude_memory = false;
        taste.worker_claude_memory = false;
        assert_eq!((taste.profile(), taste.brief()), (Profile::Balanced, brief));
        taste.apply(Profile::Aggressive);
        assert_eq!((taste.profile(), taste.claude_memory, taste.worker_claude_memory), (Profile::Aggressive, false, false), "a preset keeps them");
    }

    #[test]
    fn the_claude_memory_switches_are_the_users_own() {
        let one = TastePatch { claude_memory: Some(false), ..TastePatch::default() };
        assert_eq!(refuse_review_settings(one.review_settings()), Err("claude_memory is changed only by the user, on the Way of working page; nothing changed".to_owned()));
        let both = TastePatch { claude_memory: Some(false), worker_claude_memory: Some(false), parallel: Some(2), ..TastePatch::default() };
        assert_eq!(refuse_review_settings(both.review_settings()), Err("claude_memory and worker_claude_memory are changed only by the user, on the Way of working page; nothing changed".to_owned()));
        assert_eq!(refuse_review_settings(TasteField::WorkerClaudeMemory.review_setting()), Err("worker_claude_memory is changed only by the user, on the Way of working page; nothing changed".to_owned()));
        assert_eq!(serde_json::to_value(TasteField::ClaudeMemory).unwrap(), json!("claude_memory"));
        assert_eq!(serde_json::to_value(TasteField::WorkerClaudeMemory).unwrap(), json!("worker_claude_memory"));
    }

    #[test]
    fn manage_only_leaves_the_profile_and_the_brief_alone() {
        let mut taste = preset(Profile::Balanced);
        let brief = taste.brief();
        taste.manage_only = true;
        assert_eq!((taste.profile(), taste.brief()), (Profile::Balanced, brief));
        taste.apply(Profile::Aggressive);
        assert_eq!((taste.profile(), taste.manage_only), (Profile::Aggressive, true), "a preset keeps it");
    }

    #[test]
    fn manage_only_is_the_users_own() {
        let refused = Err("manage_only is changed only by the user, on the Way of working page; nothing changed".to_owned());
        let patch = TastePatch { manage_only: Some(true), ..TastePatch::default() };
        assert_eq!(refuse_review_settings(patch.review_settings()), refused);
        assert_eq!(refuse_review_settings(TastePatch { parallel: Some(2), ..patch }.review_settings()), refused, "mixed with an allowed field");
        assert_eq!(refuse_review_settings(TasteField::ManageOnly.review_setting()), refused);
        assert_eq!(serde_json::to_value(TasteField::ManageOnly).unwrap(), json!("manage_only"));
    }

    #[test]
    fn the_captains_default_model_resolves_for_every_project_and_a_projects_own_beats_it_there() {
        let api = TasteScope::Project { repo: "github.com/acme/api".into() };
        let sonnet = ModelRef::named("claude-sonnet-5-5");
        let glm = ModelRef { provider: Some("opencode".into()), model: "glm-5.3".into() };
        let old: Tastes = serde_json::from_value(json!({ "captains": { "parallel": 3 }, "projects": { "github.com/acme/site": { "parallel": 2 } } })).unwrap();
        assert_eq!((old.resolve(&site()).default_model, old.captains.default_model), (None, None), "tastes from before the field");

        let mut tastes = Tastes::default();
        tastes.set(&TasteScope::Captains, TastePatch { default_model: Some(sonnet.clone()), ..TastePatch::default() }).unwrap();
        assert_eq!((tastes.resolve(&site()).default_model, tastes.resolve(&api).default_model), (Some(sonnet.clone()), Some(sonnet.clone())));
        tastes.set(&site(), TastePatch { default_model: Some(glm.clone()), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "default_model": { "provider": "opencode", "model": "glm-5.3" } } }));
        assert_eq!((tastes.resolve(&site()).default_model, tastes.resolve(&api).default_model), (Some(glm.clone()), Some(sonnet.clone())), "only that project");

        tastes.set(&site(), TastePatch { default_model: Some(sonnet.clone()), parallel: Some(4), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 4 } }), "the Captains' model set on the project is dropped");
        tastes.set(&site(), TastePatch { default_model: Some(glm), ..TastePatch::default() }).unwrap();
        tastes.reset(&site(), TasteField::DefaultModel);
        assert_eq!(tastes.resolve(&site()).default_model, Some(sonnet), "a reset project inherits again");
        assert!(serde_json::to_value(&tastes.projects[SITE]).unwrap().get("default_model").is_none(), "a project's patch leaves it out");
        tastes.reset(&TasteScope::Captains, TasteField::DefaultModel);
        assert_eq!((tastes.captains.default_model.clone(), tastes.resolve(&site()).default_model), (None, None), "a reset gives the Captains none back");
        assert_eq!(serde_json::to_value(&tastes.captains).unwrap()["default_model"], Value::Null, "the Captains' is written, null included");

        assert_eq!(serde_json::to_value(TasteField::DefaultModel).unwrap(), json!("default_model"));
        assert_eq!(TasteField::DefaultModel.review_setting(), None, "the taste tools may set it");
    }

    #[test]
    fn a_default_model_that_is_no_model_name_is_refused_and_changes_nothing() {
        let evil = ModelRef { provider: None, model: "--evil".into() };
        assert_eq!(Taste { default_model: Some(evil.clone()), ..Taste::default() }.check(), Err("\"--evil\" is not a model name".into()));
        let mut tastes = Tastes::default();
        let before = tastes.clone();
        for scope in [TasteScope::Captains, site()] {
            assert_eq!(tastes.set(&scope, TastePatch { default_model: Some(evil.clone()), parallel: Some(2), ..TastePatch::default() }), Err("\"--evil\" is not a model name".into()), "{}", scope.name());
        }
        assert_eq!(tastes, before);
        assert_eq!(check_model_name("claude-sonnet-5-5"), Ok(()));
        assert_eq!(check_model_name("glm 5"), Err("\"glm 5\" is not a model name".into()));
    }

    #[test]
    fn a_default_model_leaves_the_profile_and_the_brief_alone() {
        let sonnet = Some(ModelRef::named("claude-sonnet-5-5"));
        for profile in Profile::PRESETS {
            assert_eq!(preset(profile).default_model, None);
        }
        let mut taste = preset(Profile::Economical);
        let brief = taste.brief();
        taste.default_model.clone_from(&sonnet);
        assert_eq!((taste.profile(), taste.brief()), (Profile::Economical, brief));
        taste.apply(Profile::Aggressive);
        assert_eq!((taste.profile(), taste.default_model), (Profile::Aggressive, sonnet), "a preset keeps it");
    }
}
