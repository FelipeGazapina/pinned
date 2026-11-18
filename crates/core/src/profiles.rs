//! Worker profiles of the agent team: named worker setups (skills, tools, access, model, effort and
//! what the worker delivers) that `delegate_task` names with `profile`. Called `WorkerProfile` in
//! code because `taste::Profile` already names the way-of-working presets; the wire field is
//! `worker_profiles`. The account's and each project's lists are the user's own: only the hub's
//! `set_worker_profiles` writes them, which no agent may call. An orchestrator only proposes one,
//! with `profile_propose`, into `proposed`.
//!
//! Owners: U5 (the lists, `profile_propose`), U6 (`delegate_task` with `profile`).

use crate::delegate::Request;
use crate::reviewer::Reviewer;
use crate::taste::{ModelRef, SkillRef, Tastes};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The account's worker profiles, kept in `Tastes::worker_profiles`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct WorkerProfiles {
    /// Every project's.
    pub account: Vec<WorkerProfile>,
    /// One project's own, by repo as in `github.com/acme/site`.
    pub projects: BTreeMap<String, Vec<WorkerProfile>>,
    /// What orchestrators proposed and the user has not taken or dropped yet.
    pub proposed: Vec<WorkerProfile>,
}

/// One named worker setup.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct WorkerProfile {
    pub name: String,
    #[serde(default)]
    pub skills: Vec<SkillRef>,
    /// The tools the worker is granted, as `delegate_task`'s `tools`.
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub access: ProfileAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// What the worker hands back, in one line.
    #[serde(default)]
    pub delivers: String,
    #[serde(default)]
    pub by: ProfileAuthor,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileAccess {
    // What a profile that names no access gets.
    #[default]
    ReadOnly,
    Edit,
}

/// Who wrote a profile.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileAuthor {
    #[default]
    User,
    Orchestrator,
}

impl Tastes {
    /// `HubCall::SetWorkerProfiles`: replaces the account's list, or `project`'s, or changes
    /// nothing when a profile fails `WorkerProfile::check` or two share a name. Every profile is
    /// kept as the user's, its name trimmed, whoever the sender says wrote it.
    pub fn set_worker_profiles(&mut self, project: Option<&str>, profiles: Vec<WorkerProfile>) -> Result<(), String> {
        let profiles: Vec<WorkerProfile> = profiles.into_iter().map(|p| WorkerProfile { name: p.name.trim().to_owned(), by: ProfileAuthor::User, ..p }).collect();
        let mut names = std::collections::BTreeSet::new();
        for profile in &profiles {
            profile.check()?;
            not_the_reviewer(profile)?;
            if !names.insert(profile.name.trim().to_lowercase()) {
                return Err(format!("two profiles are named {}", profile.name.trim()));
            }
        }
        let mut changed = self.worker_profiles.clone();
        match project.map(str::trim) {
            Some("") => return Err("a project's profiles need the project's repo".into()),
            Some(repo) if profiles.is_empty() => {
                changed.projects.remove(repo);
            }
            Some(repo) => {
                changed.projects.insert(repo.to_owned(), profiles);
            }
            None => changed.account = profiles,
        }
        changed.check_caps()?;
        self.worker_profiles = changed;
        Ok(())
    }
}

impl WorkerProfiles {
    /// At most `MAX_PROFILES` in any list and `MAX_PROJECT_LISTS` projects with a list of their
    /// own, which `Tastes::check` also asks of what `ImportTastes` brings.
    pub fn check_caps(&self) -> Result<(), String> {
        if self.projects.len() > MAX_PROJECT_LISTS {
            return Err(format!("at most {MAX_PROJECT_LISTS} projects have their own profiles"));
        }
        match std::iter::once(&self.account).chain(self.projects.values()).chain([&self.proposed]).any(|list| list.len() > MAX_PROFILES) {
            true => Err(format!("a list holds at most {MAX_PROFILES} profiles")),
            false => Ok(()),
        }
    }
}

impl WorkerProfile {
    pub fn check(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("every profile needs a name".into());
        }
        if name.chars().count() > MAX_NAME {
            return Err(format!("a profile's name is at most {MAX_NAME} characters"));
        }
        if self.delivers.contains('\n') {
            return Err(format!("what {name} delivers fits one line"));
        }
        if self.delivers.chars().count() > MAX_DELIVERS {
            return Err(format!("what {name} delivers is at most {MAX_DELIVERS} characters"));
        }
        if self.tools.len() > MAX_TOOLS {
            return Err(format!("{name} names at most {MAX_TOOLS} tools"));
        }
        if self.skills.len() > MAX_SKILLS {
            return Err(format!("{name} names at most {MAX_SKILLS} skills"));
        }
        if self.model.as_ref().is_some_and(|m| m.model.trim().is_empty()) {
            return Err(format!("{}'s model needs a name", self.name.trim()));
        }
        Ok(())
    }

    /// Whether every skill the profile names is one of `installed`, as the `skills` action lists them.
    pub fn check_skills(&self, installed: &[&str]) -> Result<(), String> {
        match self.skills.iter().find(|skill| !installed.contains(&skill.name.trim())) {
            Some(skill) => Err(format!("{} names {}, which is no installed skill: name one the skills action lists", self.name.trim(), skill.name.trim())),
            None => Ok(()),
        }
    }
}

/// The most profiles in one list.
pub const MAX_PROFILES: usize = 30;
/// The most projects with a list of their own.
pub const MAX_PROJECT_LISTS: usize = 200;
/// The most characters in a profile's name, trimmed.
pub const MAX_NAME: usize = 60;
/// The most characters in what a profile delivers.
pub const MAX_DELIVERS: usize = 300;
/// The most tools, and the most skills, one profile names.
pub const MAX_TOOLS: usize = 40;
pub const MAX_SKILLS: usize = 40;
/// The seed whose model, rules and rounds are `Tastes::reviewer`'s, and no list's.
pub const REVIEWER: &str = "Reviewer";

fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

fn not_the_reviewer(profile: &WorkerProfile) -> Result<(), String> {
    match same_name(&profile.name, REVIEWER) {
        true => Err("the Reviewer is set in its own section, so no profile takes its name".into()),
        false => Ok(()),
    }
}

/// The built-in profiles, which every list starts from: Explorer, Implementer, Tester and the
/// Reviewer, whose model, rules and rounds are `Tastes::reviewer`'s. A seed's model of None is the
/// ladder's rung for its kind of work.
pub fn seeds(reviewer: &Reviewer, project: Option<&str>) -> Vec<WorkerProfile> {
    let seed = |name: &str, tools: &[&str], access, delivers: &str| WorkerProfile {
        name: name.into(),
        skills: vec![],
        tools: tools.iter().map(|t| (*t).to_owned()).collect(),
        access,
        model: None,
        effort: None,
        delivers: delivers.into(),
        by: ProfileAuthor::User,
    };
    vec![
        seed("Explorer", &["Read", "Grep", "Glob"], ProfileAccess::ReadOnly, "What it found, with file:line, and no change."),
        seed("Implementer", &["Read", "Grep", "Glob", "Edit", "Write", "Bash"], ProfileAccess::Edit, "The change on its branch, with the tests it ran."),
        seed("Tester", &["Read", "Grep", "Glob", "Edit", "Write", "Bash"], ProfileAccess::Edit, "The tests it ran and their output, with screenshot proof only when the work changes a screen."),
        WorkerProfile { model: reviewer.model.clone(), ..seed(REVIEWER, &["Read", "Grep", "Glob"], ProfileAccess::ReadOnly, &reviewer_delivers(reviewer, project)) },
    ]
}

/// What the Reviewer seed delivers: the rules it reads in `project` and the rounds it sends back.
fn reviewer_delivers(reviewer: &Reviewer, project: Option<&str>) -> String {
    let own = project.and_then(|repo| reviewer.projects.get(repo.trim()));
    let replaced = |id: &str| own.is_some_and(|own| own.rules.iter().any(|rule| rule.replaces.as_deref() == Some(id)));
    let rules: Vec<&str> = reviewer.rules.iter().map(|rule| rule.id.as_str()).filter(|id| !replaced(id)).chain(own.into_iter().flat_map(|own| own.rules.iter().map(|rule| rule.id.as_str()))).collect();
    let rounds = match reviewer.rounds {
        1 => "1 round".to_owned(),
        n => format!("{n} rounds"),
    };
    match rules.is_empty() {
        true => format!("Findings on the change, sent back to the worker for at most {rounds}."),
        false => format!("Findings against rules {}, sent back to the worker for at most {rounds}.", rules.join(", ")),
    }
}

/// The profiles `delegate_task` may name in `project`, or in any project when None: the seeds,
/// then the account's list over them by name, then the project's over both. The Reviewer stays the
/// seed. Never a proposal.
pub fn approved(tastes: &Tastes, project: Option<&str>) -> Vec<WorkerProfile> {
    let mut list = seeds(&tastes.reviewer, project);
    let own = project.and_then(|repo| tastes.worker_profiles.projects.get(repo.trim()));
    for profile in tastes.worker_profiles.account.iter().chain(own.into_iter().flatten()) {
        if same_name(&profile.name, REVIEWER) {
            continue;
        }
        match list.iter_mut().find(|p| same_name(&p.name, &profile.name)) {
            Some(earlier) => *earlier = profile.clone(),
            None => list.push(profile.clone()),
        }
    }
    list
}

/// Why `resolve` found no profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unresolved {
    /// An orchestrator proposed it and the user has not taken it.
    NotApproved(String),
    Unknown(String),
}

impl fmt::Display for Unresolved {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Unresolved::NotApproved(name) => write!(f, "{name} is only proposed: the user has not approved it"),
            Unresolved::Unknown(name) => write!(f, "no profile is named {name}"),
        }
    }
}

/// The profile named `name` in `project`, or in any project when None, as `approved` lists it. A
/// name only `proposed` holds is not approved: `delegate_task` never runs a proposal.
pub fn resolve(tastes: &Tastes, project: Option<&str>, name: &str) -> Result<WorkerProfile, Unresolved> {
    if let Some(profile) = approved(tastes, project).into_iter().find(|p| same_name(&p.name, name)) {
        return Ok(profile);
    }
    match tastes.worker_profiles.proposed.iter().any(|p| same_name(&p.name, name)) {
        true => Err(Unresolved::NotApproved(name.trim().to_owned())),
        false => Err(Unresolved::Unknown(name.trim().to_owned())),
    }
}

/// `delegate_task`'s `profile`, which `parse_request` calls. U6 turns it on.
pub fn admit(request: &Request) -> Result<(), String> {
    match request.profile {
        Some(_) => Err("profile is not available in this build".into()),
        None => Ok(()),
    }
}

/// `delegate_task`'s schema for `profile`, offered once U6 turns it on.
pub fn delegate_property() -> Option<(&'static str, serde_json::Value)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile(name: &str) -> WorkerProfile {
        WorkerProfile { name: name.into(), skills: vec![], tools: vec!["Read".into()], access: ProfileAccess::ReadOnly, model: None, effort: None, delivers: "A report.".into(), by: ProfileAuthor::User }
    }

    #[test]
    fn a_taste_json_from_before_profiles_reads_with_none() {
        let tastes: Tastes = serde_json::from_value(json!({ "captains": {} })).unwrap();
        assert_eq!(tastes.worker_profiles, WorkerProfiles::default());
        let bare: WorkerProfile = serde_json::from_value(json!({ "name": "scout" })).unwrap();
        assert_eq!((bare.access, bare.by, bare.delivers.as_str()), (ProfileAccess::ReadOnly, ProfileAuthor::User, ""), "a profile that names no access is read-only");
    }

    #[test]
    fn profiles_round_trip() {
        let profiles = WorkerProfiles {
            account: vec![profile("scout")],
            projects: BTreeMap::from([("github.com/acme/site".into(), vec![WorkerProfile { model: Some(ModelRef { provider: None, model: "glm-5.3".into() }), effort: Some("high".into()), access: ProfileAccess::Edit, ..profile("fixer") }])]),
            proposed: vec![WorkerProfile { by: ProfileAuthor::Orchestrator, ..profile("tester") }],
        };
        let value = serde_json::to_value(&profiles).unwrap();
        assert_eq!(value["account"][0]["access"], "read_only");
        assert_eq!(value["proposed"][0]["by"], "orchestrator");
        assert_eq!(serde_json::from_value::<WorkerProfiles>(value).unwrap(), profiles);
    }

    #[test]
    fn set_worker_profiles_stores_a_list_and_refuses_a_bad_one_whole() {
        let mut tastes = Tastes::default();
        tastes.set_worker_profiles(None, vec![profile("scout")]).unwrap();
        tastes.set_worker_profiles(Some("github.com/acme/site"), vec![profile("fixer")]).unwrap();
        assert_eq!((tastes.worker_profiles.account.len(), tastes.worker_profiles.projects.len()), (1, 1));
        let before = tastes.clone();
        assert_eq!(tastes.set_worker_profiles(None, vec![profile("a"), profile("A")]), Err("two profiles are named A".into()));
        assert_eq!(tastes.set_worker_profiles(None, vec![profile(" ")]), Err("every profile needs a name".into()));
        assert_eq!(tastes.set_worker_profiles(None, vec![WorkerProfile { delivers: "a\nb".into(), ..profile("x") }]), Err("what x delivers fits one line".into()));
        assert_eq!(tastes, before);
        tastes.set_worker_profiles(Some("github.com/acme/site"), vec![]).unwrap();
        assert!(tastes.worker_profiles.projects.is_empty());
    }

    #[test]
    fn the_users_list_is_kept_as_the_users_with_names_trimmed() {
        let mut tastes = Tastes::default();
        let sent = vec![WorkerProfile { by: ProfileAuthor::Orchestrator, ..profile("  scout ") }, profile("fixer\t")];
        tastes.set_worker_profiles(None, sent.clone()).unwrap();
        tastes.set_worker_profiles(Some("github.com/acme/site"), sent).unwrap();
        for list in [&tastes.worker_profiles.account, &tastes.worker_profiles.projects["github.com/acme/site"]] {
            assert_eq!(list.iter().map(|p| (p.name.as_str(), p.by)).collect::<Vec<_>>(), [("scout", ProfileAuthor::User), ("fixer", ProfileAuthor::User)]);
        }
    }

    #[test]
    fn every_cap_takes_its_limit_and_refuses_one_past_it() {
        let set = |profiles: Vec<WorkerProfile>| Tastes::default().set_worker_profiles(None, profiles);
        let many = |n: usize| (0..n).map(|i| format!("t{i}")).collect::<Vec<String>>();
        let skills = |n: usize| (0..n).map(|i| SkillRef { name: format!("s{i}"), enabled: true }).collect::<Vec<_>>();
        for (at, past, refused) in [
            (profile(&"n".repeat(60)), profile(&"n".repeat(61)), "a profile's name is at most 60 characters"),
            (profile(&format!("  {}  ", "é".repeat(60))), profile(&"é".repeat(61)), "a profile's name is at most 60 characters"),
            (WorkerProfile { delivers: "d".repeat(300), ..profile("x") }, WorkerProfile { delivers: "d".repeat(301), ..profile("x") }, "what x delivers is at most 300 characters"),
            (WorkerProfile { tools: many(40), ..profile("x") }, WorkerProfile { tools: many(41), ..profile("x") }, "x names at most 40 tools"),
            (WorkerProfile { skills: skills(40), ..profile("x") }, WorkerProfile { skills: skills(41), ..profile("x") }, "x names at most 40 skills"),
        ] {
            assert_eq!(set(vec![at]), Ok(()), "{refused}");
            assert_eq!(set(vec![past]), Err(refused.into()));
        }
        let list = |n: usize| (0..n).map(|i| profile(&format!("p{i}"))).collect::<Vec<_>>();
        assert_eq!(set(list(30)), Ok(()));
        assert_eq!(set(list(31)), Err("a list holds at most 30 profiles".into()));
        let mut tastes = Tastes::default();
        assert_eq!(tastes.set_worker_profiles(Some("github.com/acme/site"), list(31)), Err("a list holds at most 30 profiles".into()));
        for i in 0..200 {
            tastes.set_worker_profiles(Some(&format!("github.com/acme/p{i}")), list(1)).unwrap();
        }
        let before = tastes.clone();
        assert_eq!(tastes.set_worker_profiles(Some("github.com/acme/p200"), list(1)), Err("at most 200 projects have their own profiles".into()));
        assert_eq!(tastes, before, "a refused list changed nothing");
        tastes.set_worker_profiles(Some("github.com/acme/p0"), list(30)).unwrap();
        // Tastes::check, which ImportTastes runs, refuses what the user's own write refuses.
        assert_eq!(tastes.check(), Ok(()));
        let mut imported = tastes.clone();
        imported.worker_profiles.projects.insert("github.com/acme/p200".into(), list(1));
        assert_eq!(imported.check(), Err("at most 200 projects have their own profiles".into()));
        let mut imported = tastes.clone();
        imported.worker_profiles.account = list(31);
        assert_eq!(imported.check(), Err("a list holds at most 30 profiles".into()));
        let mut imported = tastes;
        imported.worker_profiles.account = vec![WorkerProfile { tools: many(41), ..profile("x") }];
        assert_eq!(imported.check(), Err("x names at most 40 tools".into()));
    }

    fn names(profiles: &[WorkerProfile]) -> Vec<&str> {
        profiles.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn the_seeds_are_there_on_empty_tastes() {
        let tastes = Tastes::default();
        for project in [None, Some("github.com/acme/site")] {
            assert_eq!(names(&approved(&tastes, project)), ["Explorer", "Implementer", "Tester", "Reviewer"], "{project:?}");
        }
        let explorer = resolve(&tastes, None, "explorer").unwrap();
        assert_eq!((explorer.access, explorer.model, explorer.by), (ProfileAccess::ReadOnly, None, ProfileAuthor::User));
        assert_eq!(resolve(&tastes, None, "Implementer").unwrap().access, ProfileAccess::Edit);
        let tester = resolve(&tastes, None, "Tester").unwrap();
        assert!(tester.delivers.contains("screenshot") && tester.delivers.contains("only when the work changes a screen"), "{}", tester.delivers);
        for seed in seeds(&tastes.reviewer, None) {
            seed.check().unwrap();
        }
    }

    #[test]
    fn the_reviewer_seed_reads_its_model_and_rounds_from_the_reviewer() {
        let mut tastes = Tastes::default();
        let seed = resolve(&tastes, None, "Reviewer");
        assert_eq!(seed.clone().map(|p| (p.model, p.access)), Ok((None, ProfileAccess::ReadOnly)), "no model set: the ladder picks it");
        assert!(seed.unwrap().delivers.contains("2 rounds"));
        let opus = ModelRef { provider: None, model: "claude-opus-5-5".into() };
        tastes.reviewer.model = Some(opus.clone());
        tastes.reviewer.rounds = 4;
        tastes.reviewer.rules = vec![crate::reviewer::Rule { id: "A1".into(), text: "Tests first.".into(), replaces: None }];
        let seed = resolve(&tastes, None, "reviewer").unwrap();
        assert_eq!(seed.model, Some(opus));
        assert!(seed.delivers.contains("4 rounds") && seed.delivers.contains("A1"), "{}", seed.delivers);
        // A list of the user's names no second Reviewer: the Reviewer section sets it.
        assert_eq!(tastes.set_worker_profiles(None, vec![profile("reviewer")]), Err("the Reviewer is set in its own section, so no profile takes its name".into()));
        tastes.worker_profiles.account.push(profile("Reviewer"));
        assert_eq!(names(&approved(&tastes, None)).iter().filter(|n| n.eq_ignore_ascii_case("reviewer")).count(), 1);
        assert_eq!(resolve(&tastes, None, "Reviewer").unwrap().model, tastes.reviewer.model);
    }

    #[test]
    fn a_project_profile_overrides_an_account_profile_of_the_same_name() {
        let mut tastes = Tastes::default();
        tastes.set_worker_profiles(None, vec![profile("scout"), WorkerProfile { delivers: "The account's tests.".into(), ..profile("Tester") }]).unwrap();
        tastes.set_worker_profiles(Some("github.com/acme/site"), vec![WorkerProfile { access: ProfileAccess::Edit, delivers: "The site's scout.".into(), ..profile("Scout") }]).unwrap();
        let delivers = |project, name| resolve(&tastes, project, name).map(|p| p.delivers);
        assert_eq!(delivers(None, "scout"), Ok("A report.".into()));
        assert_eq!(delivers(Some("github.com/acme/other"), "scout"), Ok("A report.".into()));
        let site = resolve(&tastes, Some("github.com/acme/site"), "scout").unwrap();
        assert_eq!((site.delivers.as_str(), site.access), ("The site's scout.", ProfileAccess::Edit));
        assert_eq!(names(&approved(&tastes, Some("github.com/acme/site"))), ["Explorer", "Implementer", "Tester", "Reviewer", "Scout"]);
        // The user's edit of a seed stands in for it.
        assert_eq!(delivers(Some("github.com/acme/site"), "tester"), Ok("The account's tests.".into()));
    }

    #[test]
    fn a_proposed_profile_cannot_be_resolved() {
        let mut tastes = Tastes::default();
        tastes.worker_profiles.proposed.push(WorkerProfile { by: ProfileAuthor::Orchestrator, ..profile("fixer") });
        assert_eq!(resolve(&tastes, None, "fixer"), Err(Unresolved::NotApproved("fixer".into())));
        assert_eq!(resolve(&tastes, Some("github.com/acme/site"), "Fixer"), Err(Unresolved::NotApproved("Fixer".into())));
        assert_eq!(resolve(&tastes, None, "fixer").unwrap_err().to_string(), "fixer is only proposed: the user has not approved it");
        assert_eq!(resolve(&tastes, None, "ghost"), Err(Unresolved::Unknown("ghost".into())));
        assert!(!names(&approved(&tastes, None)).contains(&"fixer"));
        // Once the user takes it into a list, it resolves.
        tastes.set_worker_profiles(None, vec![profile("fixer")]).unwrap();
        assert_eq!(resolve(&tastes, None, "fixer").unwrap().by, ProfileAuthor::User);
    }

    #[test]
    fn a_proposal_lands_only_in_proposed() {
        let mut tastes = Tastes::default();
        tastes.propose_worker_profile(profile("fixer")).unwrap();
        tastes.propose_worker_profile(WorkerProfile { delivers: "A fix.".into(), ..profile("Fixer") }).unwrap();
        tastes.propose_worker_profile(WorkerProfile { delivers: "A fix.".into(), ..profile(" Fixer ") }).unwrap();
        assert_eq!(tastes.worker_profiles.proposed, vec![WorkerProfile { delivers: "A fix.".into(), by: ProfileAuthor::Orchestrator, ..profile("Fixer") }], "the same name replaces, trimmed, by the orchestrator");
        assert_eq!((tastes.worker_profiles.account.len(), tastes.worker_profiles.projects.len()), (0, 0));
        let before = tastes.clone();
        assert_eq!(tastes.propose_worker_profile(profile(" ")), Err("every profile needs a name".into()));
        assert_eq!(tastes, before);
    }

    #[test]
    fn a_profile_names_installed_skills_only() {
        let skilled = WorkerProfile { skills: vec![SkillRef { name: "pstack:unslop".into(), enabled: true }], ..profile("writer") };
        assert_eq!(skilled.check_skills(&["pstack:unslop", "bro"]), Ok(()));
        assert_eq!(skilled.check_skills(&["unslop"]), Err("writer names pstack:unslop, which is no installed skill: name one the skills action lists".into()));
    }

    /// The Way of working page draws the seeds from `PROFILE_SEEDS` in `ui/gui-profiles.js`; the
    /// Reviewer's row it builds from `Tastes.reviewer`. A seed changed here and not there fails.
    #[test]
    fn the_pages_seeds_match_the_built_in_ones() {
        let js = include_str!("../../../ui/gui-profiles.js");
        let start = js.find("const PROFILE_SEEDS = ").expect("ui/gui-profiles.js holds const PROFILE_SEEDS") + "const PROFILE_SEEDS = ".len();
        let end = start + js[start..].find("];").expect("PROFILE_SEEDS ends with ];") + 1;
        let mirror: Vec<WorkerProfile> = serde_json::from_str(&js[start..end]).expect("PROFILE_SEEDS is a JSON array of profiles");
        let built_in: Vec<WorkerProfile> = seeds(&Reviewer::default(), None).into_iter().filter(|p| p.name != REVIEWER).collect();
        assert_eq!(mirror, built_in);
    }
}
