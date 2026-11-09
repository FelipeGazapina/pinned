//! How the user wants the agents to work: which model a worker starts on and climbs to, how many
//! run at once, and what they decide alone. The global orchestrator and the Capitães each have a
//! `Taste`; a project changes some of the Capitães' knobs with a `TastePatch`. The hub keeps an
//! account's `Tastes`; a machine keeps the last ones it read in `taste.json`, and a signed-out app
//! keeps its own there.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

const FILE: &str = "taste.json";
const MAX_PARALLEL: u32 = 8;
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
    /// Workers at once, 1 to 8.
    pub parallel: u32,
    pub autonomy: Autonomy,
    pub review_diffs: bool,
    pub second_opinion: bool,
    pub skills: Vec<SkillRef>,
    pub context: Vec<String>,
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
                skills: vec![],
                context: vec![],
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
                skills: vec![],
                context: vec![],
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
    }
}

impl Default for Taste {
    fn default() -> Taste {
        balanced()
    }
}

impl Taste {
    pub fn profile(&self) -> Profile {
        let knobs = Taste { skills: vec![], context: vec![], ..self.clone() };
        Profile::PRESETS.into_iter().find(|p| p.preset().as_ref() == Some(&knobs)).unwrap_or(Profile::Custom)
    }

    /// Sets every knob but skills and context to the preset's. `Custom` changes nothing.
    pub fn apply(&mut self, profile: Profile) {
        if let Some(preset) = profile.preset() {
            *self = Taste { skills: std::mem::take(&mut self.skills), context: std::mem::take(&mut self.context), ..preset };
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
        }
    }
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

You have three tools for it. taste_get(scope) reads a scope: the settings its sessions get (effective), what the scope sets itself (own), and the brief text they start with. taste_set(scope, patch) changes the fields the patch names and keeps the rest; a project's field set to the Captains' value goes back to inheriting it. taste_reset(scope, field) makes a project inherit the field again, or gives the orchestrator and the Captains its default back. A scope is orchestrator, captains or project:<repo>. The fields are strategy (cheap_with_guidance, escalate_on_failure or strongest_first), ladder (models, cheapest first), hard_model, parallel (1 to 8), autonomy (ask_first, decide_and_tell or report_at_end), review_diffs, second_opinion, skills ({{name, enabled}}) and context (notes every turn carries). The Way of working page shows each change as soon as it lands, and new sessions start with it.

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
    // Read as plain JSON, so this keeps working once `orchestration::Style` is gone.
    let old: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(crate::orchestration::FILE)).ok()?).ok()?;
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
        let wide = TastePatch { parallel: Some(9), autonomy: Some(Autonomy::ReportAtEnd), ..TastePatch::default() };
        assert_eq!(tastes.set(&TasteScope::Captains, wide.clone()), Err("parallel workers go from 1 to 8".into()));
        assert_eq!(tastes.set(&site(), wide), Err("parallel workers go from 1 to 8".into()));
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
            },
            "captains": {
                "strategy": "escalate_on_failure", "ladder": [{ "model": "glm-5.3" }, { "model": "claude-sonnet-5-5" }, opus], "hard_model": opus,
                "parallel": 3, "autonomy": "decide_and_tell", "review_diffs": true, "second_opinion": false, "skills": [], "context": [],
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
    fn check_refuses_an_empty_ladder_a_nameless_model_and_more_than_eight_workers() {
        assert_eq!(Taste::default().check(), Ok(()));
        assert_eq!(Taste { ladder: vec![], ..Taste::default() }.check(), Err("the model ladder needs at least one model".into()));
        assert_eq!(Taste { hard_model: Some(ModelRef::named(" ")), ..Taste::default() }.check(), Err("every model needs a name".into()));
        assert_eq!(Taste { parallel: 9, ..Taste::default() }.check(), Err("parallel workers go from 1 to 8".into()));
        assert_eq!(Taste { parallel: 0, ..Taste::default() }.check(), Err("parallel workers go from 1 to 8".into()));
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
        assert_eq!(refused, Err("parallel workers go from 1 to 8".into()));
        assert_eq!(load(Some(dir.path())), Some(changed));
    }
}

