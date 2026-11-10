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
    /// The effort a worker gets when the call names none. None leaves it to the model.
    pub default_effort: Option<Effort>,
    /// The most effort a call may ask for.
    pub max_effort: Effort,
    /// What one worker may spend, in US cents, as its transcript prices it. 0 sets no cap.
    pub budget_cents: u32,
    /// Minutes a worker runs before pinned stops it: the default and the ceiling of `limitMs`.
    pub time_limit_min: u32,
    /// A box's budget in US cents: the default and the ceiling of its `budget_usd`. 0 sets neither.
    pub box_budget_cents: u32,
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
    DefaultEffort,
    MaxEffort,
    BudgetCents,
    TimeLimitMin,
    BoxBudgetCents,
}

/// An account's tastes.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct Tastes {
    pub orchestrator: Taste,
    pub captains: Taste,
    /// Each project's changes over `captains`, by repo.
    pub projects: BTreeMap<String, TastePatch>,
}

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
                parallel: 6,
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
        parallel: 3,
        autonomy: Autonomy::DecideAndTell,
        review_diffs: true,
        second_opinion: false,
        skills: vec![],
        context: vec![],
        tier: Tier::High,
        default_effort: None,
        max_effort: Effort::Max,
        budget_cents: 0,
        time_limit_min: DEFAULT_TIME_LIMIT_MIN,
        box_budget_cents: 0,
    }
}

impl Default for Taste {
    fn default() -> Taste {
        balanced()
    }
}

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
        if !(1..=MAX_PARALLEL).contains(&self.parallel) {
            return Err(format!("parallel workers go from 1 to {MAX_PARALLEL}"));
        }
        if !(1..=MAX_TIME_LIMIT_MIN).contains(&self.time_limit_min) {
            return Err(format!("the time limit goes from 1 to {MAX_TIME_LIMIT_MIN} minutes"));
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
            TasteField::DefaultEffort => self.default_effort = default.default_effort,
            TasteField::MaxEffort => self.max_effort = default.max_effort,
            TasteField::BudgetCents => self.budget_cents = default.budget_cents,
            TasteField::TimeLimitMin => self.time_limit_min = default.time_limit_min,
            TasteField::BoxBudgetCents => self.box_budget_cents = default.box_budget_cents,
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
    /// above the tier, a call that names no model under a tier cap, and an effort above the most.
    /// An effort outside the five levels, such as Cursor's `fast`, passes unranked.
    pub fn admit(&self, model: Option<&str>, effort: Option<&str>) -> Result<Option<String>, String> {
        if self.tier < Tier::High {
            let tier = self.tier.name();
            let Some(model) = model else {
                return Err(format!("name a model: this project allows models up to the {tier} tier, and a worker without one runs on its account's default. Pick a {tier} model from orchestrator_capabilities, or raise Tier {RAISE}"));
            };
            let cost = crate::delegate::cost_of(model);
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

    /// Why a worker that spent `cost_usd` must stop, if it passed the budget.
    pub fn over_budget(&self, cost_usd: Option<f64>) -> Option<String> {
        let spent = cost_usd?;
        (self.budget_cents > 0 && spent * 100.0 >= f64::from(self.budget_cents)).then(|| format!("pinned stopped it after it spent US$ {spent:.2}, its budget of {}", usd(self.budget_cents)))
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

fn usd(cents: u32) -> String {
    format!("US$ {}.{:02}", cents / 100, cents % 100)
}

impl TastePatch {
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
        if self.default_effort.is_some() {
            taste.default_effort = self.default_effort;
        }
        put(&self.max_effort, &mut taste.max_effort);
        put(&self.budget_cents, &mut taste.budget_cents);
        put(&self.time_limit_min, &mut taste.time_limit_min);
        put(&self.box_budget_cents, &mut taste.box_budget_cents);
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
        take(newer.default_effort, &mut self.default_effort);
        take(newer.max_effort, &mut self.max_effort);
        take(newer.budget_cents, &mut self.budget_cents);
        take(newer.time_limit_min, &mut self.time_limit_min);
        take(newer.box_budget_cents, &mut self.box_budget_cents);
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
        if self.default_effort.is_some() && self.default_effort == base.default_effort {
            self.default_effort = None;
        }
        drop_same(&mut self.max_effort, &base.max_effort);
        drop_same(&mut self.budget_cents, &base.budget_cents);
        drop_same(&mut self.time_limit_min, &base.time_limit_min);
        drop_same(&mut self.box_budget_cents, &base.box_budget_cents);
    }

    pub fn is_empty(&self) -> bool {
        *self == TastePatch::default()
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
            TasteField::DefaultEffort => self.default_effort = None,
            TasteField::MaxEffort => self.max_effort = None,
            TasteField::BudgetCents => self.budget_cents = None,
            TasteField::TimeLimitMin => self.time_limit_min = None,
            TasteField::BoxBudgetCents => self.box_budget_cents = None,
        }
    }
}

impl Tastes {
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
    /// project's patch keeps only the fields that differ from the Capitães'.
    pub fn set(&mut self, scope: &TasteScope, patch: TastePatch) -> Result<(), String> {
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

    /// Whether every scope, each project resolved, passes `Taste::check`.
    pub fn check(&self) -> Result<(), String> {
        self.orchestrator.check()?;
        self.captains.check()?;
        self.projects.keys().try_for_each(|repo| self.resolve(&TasteScope::Project { repo: repo.clone() }).check())
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

You have three tools for it. taste_get(scope) reads a scope: the settings its sessions get (effective), what the scope sets itself (own), and the brief text they start with. taste_set(scope, patch) changes the fields the patch names and keeps the rest; a project's field set to the Captains' value goes back to inheriting it. taste_reset(scope, field) makes a project inherit the field again, or gives the orchestrator and the Captains its default back. A scope is orchestrator, captains, or project: followed by the project's repo, as in project:github.com/acme/site. The fields are strategy (cheap_with_guidance, escalate_on_failure or strongest_first), ladder (models, cheapest first), hard_model, parallel (workers at once, 1 to 100), autonomy (ask_first, decide_and_tell or report_at_end), review_diffs, second_opinion, skills ({{name, enabled}}) and context (notes every turn carries), and the delegation limits pinned enforces: tier (low, mid or high), default_effort and max_effort (low, medium, high, xhigh or max), budget_cents (per worker, 0 for no cap), time_limit_min (1 to 10080) and box_budget_cents (per box, 0 for none). The Way of working page shows each change as soon as it lands, and new sessions start with it.

Start with taste_get(\"{name}\"). Then interview me: one short question at a time, each with two or three answers to pick from, and change nothing yet. When you know what to change, list the changes in plain words, apply them with taste_set or taste_reset once I agree, and read the scope again to show me the new brief.",
        who = scope.label(),
    )
}

/// The tastes in `dir`'s `taste.json`. Without one, an `orchestration.json` from before tastes
/// existed seeds the Capitães with its cost style's preset, written to `taste.json` so the seed
/// outlives the style. None when neither is there, or when `taste.json` can't be read, which
/// the seed must not overwrite.
pub fn load(dir: Option<&Path>) -> Option<Tastes> {
    let dir = dir?;
    match std::fs::read(dir.join(FILE)) {
        Ok(bytes) => {
            return serde_json::from_slice(&bytes)
                .map_err(|e| eprintln!("pinned: {} is unreadable, so the default taste applies: {e}", dir.join(FILE).display()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SITE: &str = "github.com/acme/site";

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
        assert_eq!((taste.parallel, taste.autonomy, &taste.skills, &taste.context), (6, Autonomy::ReportAtEnd, &skills, &context));
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
        let patch = TastePatch { parallel: Some(5), autonomy: Some(Autonomy::ReportAtEnd), ..TastePatch::default() };
        let tastes = Tastes { captains: preset(Profile::Economical), projects: BTreeMap::from([(SITE.to_owned(), patch)]), ..Tastes::default() };
        let expected = Taste {
            strategy: Strategy::CheapWithGuidance,
            ladder: vec![ModelRef::named("deepseek-v4.1-flash"), ModelRef::named("glm-5.3"), ModelRef::named("claude-opus-5-5")],
            hard_model: Some(ModelRef::named("claude-opus-5-5")),
            parallel: 5,
            autonomy: Autonomy::ReportAtEnd,
            review_diffs: true,
            second_opinion: false,
            skills: vec![],
            context: vec![],
            tier: Tier::High,
            default_effort: None,
            max_effort: Effort::Max,
            budget_cents: 0,
            time_limit_min: 480,
            box_budget_cents: 0,
        };
        assert_eq!(tastes.resolve(&site()), expected);
        assert_eq!(tastes.resolve(&TasteScope::Project { repo: "github.com/acme/api".into() }).parallel, 2, "a project with no patch is the Capitães'");
        assert_eq!(tastes.resolve(&TasteScope::Orchestrator).parallel, 3);
    }

    #[test]
    fn a_project_field_set_to_the_inherited_value_is_dropped_and_an_empty_patch_goes() {
        let mut tastes = Tastes::default();
        tastes.set(&site(), TastePatch { parallel: Some(5), autonomy: Some(Autonomy::DecideAndTell), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 5 } }));
        tastes.set(&site(), TastePatch { second_opinion: Some(true), ..TastePatch::default() }).unwrap();
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "parallel": 5, "second_opinion": true } }), "a later set merges");
        tastes.set(&site(), TastePatch { parallel: Some(3), second_opinion: Some(false), ..TastePatch::default() }).unwrap();
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
        tastes.set(&site(), TastePatch { parallel: Some(5), review_diffs: Some(false), ..TastePatch::default() }).unwrap();
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
            "Run up to 3 workers at once.
Start each worker on glm-5.3, and when it fails twice move it one rung up the ladder: claude-sonnet-5-5, then claude-opus-5-5.
Send hard work, such as architecture calls or a bug whose cause is unknown, straight to claude-opus-5-5.
Decide on your own and tell the user what you decided.
Review each worker's diff before you accept its work."
        );
        assert_eq!(
            preset(Profile::Aggressive).brief(),
            "Run up to 6 workers at once.
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
        let patch = TastePatch { parallel: Some(5), hard_model: Some(ModelRef { provider: Some("cursor".into()), model: "grok-4.7".into() }), ..TastePatch::default() };
        let tastes = Tastes { orchestrator: preset(Profile::Aggressive), projects: BTreeMap::from([(SITE.to_owned(), patch)]), ..Tastes::default() };
        let opus = json!({ "model": "claude-opus-5-5" });
        let json = json!({
            "orchestrator": {
                "strategy": "strongest_first", "ladder": [opus], "hard_model": opus, "parallel": 6, "autonomy": "report_at_end",
                "review_diffs": false, "second_opinion": true, "skills": [], "context": [],
                "tier": "high", "default_effort": null, "max_effort": "max", "budget_cents": 0, "time_limit_min": 480, "box_budget_cents": 0,
            },
            "captains": {
                "strategy": "escalate_on_failure", "ladder": [{ "model": "glm-5.3" }, { "model": "claude-sonnet-5-5" }, opus], "hard_model": opus,
                "parallel": 3, "autonomy": "decide_and_tell", "review_diffs": true, "second_opinion": false, "skills": [], "context": [],
                "tier": "high", "default_effort": null, "max_effort": "max", "budget_cents": 0, "time_limit_min": 480, "box_budget_cents": 0,
            },
            "projects": { "github.com/acme/site": { "hard_model": { "provider": "cursor", "model": "grok-4.7" }, "parallel": 5 } },
        });
        assert_eq!(serde_json::to_value(&tastes).unwrap(), json);
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
    fn a_change_to_the_file_lands_and_a_refused_one_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let changed = change(Some(dir.path()), |tastes| tastes.set(&site(), TastePatch { parallel: Some(5), ..TastePatch::default() })).unwrap();
        let written: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("taste.json")).unwrap()).unwrap();
        assert_eq!(written["projects"], json!({ "github.com/acme/site": { "parallel": 5 } }));
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
            Limits { parallel: 3, tier: Tier::Low, default_effort: Some(Effort::Medium), max_effort: Effort::Max, budget_cents: 500, time_limit_min: 60, box_budget_cents: 0 }
        );
        assert_eq!(tastes.resolve(&TasteScope::Project { repo: "github.com/acme/api".into() }).limits().tier, Tier::Mid, "a project with no patch has the Captains' limits");
        assert_eq!(serde_json::to_value(&tastes.projects).unwrap(), json!({ "github.com/acme/site": { "tier": "low", "default_effort": "medium", "time_limit_min": 60 } }));
        tastes.reset(&site(), TasteField::Tier);
        assert_eq!(tastes.resolve(&site()).limits().tier, Tier::Mid, "a reset project inherits the tier again");

        let mut captains = tastes.captains.clone();
        assert_eq!(captains.profile(), Profile::Balanced, "limits other than workers at once make no preset Custom");
        captains.apply(Profile::Aggressive);
        assert_eq!((captains.parallel, captains.tier, captains.budget_cents), (6, Tier::Mid, 500), "a preset sets workers at once and keeps the other limits");
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
        let limits = Limits { tier: Tier::Mid, default_effort: Some(Effort::Medium), max_effort: Effort::High, ..Limits::default() };
        assert_eq!(limits.admit(Some("glm-5.3"), None), Ok(Some("medium".into())));
        assert_eq!(limits.admit(Some("deepseek-v4.1-flash"), Some("high")), Ok(Some("high".into())));
        assert_eq!(limits.admit(Some("composer-2.5"), Some("fast")), Ok(Some("fast".into())), "a level outside the five passes");
        assert_eq!(limits.admit(Some("kimi-k3.1-pro"), None), Err("kimi-k3.1-pro is a high-tier model; this project allows up to mid. Pick a mid model or raise Tier in Settings › Way of working".into()));
        assert_eq!(limits.admit(Some("glm-5.3"), Some("xhigh")), Err("effort xhigh is above high, the most this project allows. Ask for high or less, or raise Max effort in Settings › Way of working".into()));
        assert_eq!(Limits::default().admit(None, None), Ok(None), "with no tier cap a call may leave the model and the effort to the account");
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
            "pinned enforces these delegation limits on delegate_task: at most 3 workers at once, models up to the high tier, effort up to max, 480 minutes per worker. A call past them is refused, and a worker past its time or budget is stopped; if a limit blocks you, tell the user to raise it in Settings › Way of working rather than working around it. The tier covers the model each worker runs on, not the models it picks for its own subagents."
        );
        let tight = Limits { parallel: 1, tier: Tier::Low, default_effort: Some(Effort::Low), max_effort: Effort::Medium, budget_cents: 150, time_limit_min: 45, box_budget_cents: 1000 };
        assert_eq!(
            tight.hint(),
            "pinned enforces these delegation limits on delegate_task: one worker at a time, models up to the low tier, effort up to medium (low when a call names none), 45 minutes per worker, US$ 1.50 per worker, US$ 10.00 per box. A call past them is refused, and a worker past its time or budget is stopped; if a limit blocks you, tell the user to raise it in Settings › Way of working rather than working around it. The tier covers the model each worker runs on, not the models it picks for its own subagents."
        );
        assert_eq!(tight.over_budget(Some(1.49)), None);
        assert_eq!(tight.over_budget(None), None, "an unpriced worker can't be held to a budget");
        assert_eq!(tight.over_budget(Some(1.5)), Some("pinned stopped it after it spent US$ 1.50, its budget of US$ 1.50".into()));
        assert_eq!(Limits::default().over_budget(Some(1000.0)), None, "no budget, no stop");
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
        assert_eq!(load(Some(fresh.path())).map(|t| (t.captains.profile(), t.captains.parallel)), Some((Profile::Aggressive, 6)), "the default cap lowers nothing, and the style still seeds");
        assert!(!fresh.path().join("orchestration.json").exists());
    }
}
