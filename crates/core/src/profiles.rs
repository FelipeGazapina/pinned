//! Worker profiles of the agent team: named worker setups (skills, tools, access, model, effort and
//! what the worker delivers) that `delegate_task` names with `profile`. Called `WorkerProfile` in
//! code because `taste::Profile` already names the way-of-working presets; the wire field is
//! `worker_profiles`. The account's and each project's lists are the user's own: only the hub's
//! `set_worker_profiles` writes them, which no agent may call. An orchestrator only proposes one,
//! with `profile_propose`, into `proposed`.
//!
//! Owners: U5 (the lists, `profile_propose`), U6 (`delegate_task` with `profile`).

use crate::taste::{ModelRef, SkillRef, Tastes};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    /// Who the worker is, put ahead of its task. Several lines, at most `MAX_PROMPT` characters.
    #[serde(default)]
    pub prompt: String,
}

/// The most characters a profile's `prompt` may have.
pub const MAX_PROMPT: usize = 8000;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileAccess {
    ReadOnly,
    #[default]
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
    /// nothing when a profile fails `WorkerProfile::check` or two share a name.
    pub fn set_worker_profiles(&mut self, project: Option<&str>, profiles: Vec<WorkerProfile>) -> Result<(), String> {
        let mut names = std::collections::BTreeSet::new();
        for profile in &profiles {
            profile.check()?;
            if !names.insert(profile.name.trim().to_lowercase()) {
                return Err(format!("two profiles are named {}", profile.name.trim()));
            }
        }
        match project.map(str::trim) {
            Some("") => return Err("a project's profiles need the project's repo".into()),
            Some(repo) if profiles.is_empty() => {
                self.worker_profiles.projects.remove(repo);
            }
            Some(repo) => {
                self.worker_profiles.projects.insert(repo.to_owned(), profiles);
            }
            None => self.worker_profiles.account = profiles,
        }
        Ok(())
    }
}

impl WorkerProfile {
    pub fn check(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("every profile needs a name".into());
        }
        if self.delivers.contains('\n') {
            return Err(format!("what {} delivers fits one line", self.name.trim()));
        }
        if self.model.as_ref().is_some_and(|m| m.model.trim().is_empty()) {
            return Err(format!("{}'s model needs a name", self.name.trim()));
        }
        if self.prompt.chars().count() > MAX_PROMPT {
            return Err(format!("{}'s prompt is over {MAX_PROMPT} characters", self.name.trim()));
        }
        Ok(())
    }
}

/// `delegate_task`'s schema for `profile`.
pub fn delegate_property() -> Option<(&'static str, serde_json::Value)> {
    Some((
        "profile",
        serde_json::json!({
            "type": "string",
            "description": "One of the user's worker profiles, by name: taste_get lists them under worker_profiles, this project's first, then the account's. The profile fills in the model, effort and tools this call leaves out, makes the worker read-only when the profile is read-only, and puts the profile's prompt ahead of task. An unknown name is refused with the names available.",
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile(name: &str) -> WorkerProfile {
        WorkerProfile { name: name.into(), skills: vec![], tools: vec!["Read".into()], access: ProfileAccess::ReadOnly, model: None, effort: None, delivers: "A report.".into(), by: ProfileAuthor::User, prompt: String::new() }
    }

    #[test]
    fn a_taste_json_from_before_profiles_reads_with_none() {
        let tastes: Tastes = serde_json::from_value(json!({ "captains": {} })).unwrap();
        assert_eq!(tastes.worker_profiles, WorkerProfiles::default());
        let bare: WorkerProfile = serde_json::from_value(json!({ "name": "scout" })).unwrap();
        assert_eq!((bare.access, bare.by, bare.delivers.as_str()), (ProfileAccess::Edit, ProfileAuthor::User, ""));
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
    fn a_profile_without_a_prompt_reads_empty_and_one_over_the_cap_is_refused() {
        let bare: WorkerProfile = serde_json::from_value(json!({ "name": "scout", "access": "read_only" })).unwrap();
        assert_eq!(bare.prompt, "");
        let profiles = WorkerProfiles { account: vec![WorkerProfile { prompt: "You map code.\nYou never edit it.".into(), ..profile("scout") }], ..WorkerProfiles::default() };
        let value = serde_json::to_value(&profiles).unwrap();
        assert_eq!(value["account"][0]["prompt"], "You map code.\nYou never edit it.");
        assert_eq!(serde_json::from_value::<WorkerProfiles>(value).unwrap(), profiles);
        let mut tastes = Tastes::default();
        tastes.set_worker_profiles(None, vec![profile("fixer")]).unwrap();
        let before = tastes.clone();
        let long = WorkerProfile { prompt: "é".repeat(MAX_PROMPT + 1), ..profile("scout") };
        assert_eq!(tastes.set_worker_profiles(None, vec![long.clone()]), Err("scout's prompt is over 8000 characters".into()));
        assert_eq!(tastes, before);
        tastes.worker_profiles.account.push(long);
        assert_eq!(tastes.check(), Err("scout's prompt is over 8000 characters".into()), "so an import refuses it too");
        let most = WorkerProfile { prompt: "é".repeat(MAX_PROMPT), ..profile("scout") };
        assert!(before.clone().set_worker_profiles(None, vec![most]).is_ok(), "the cap counts characters, not bytes");
    }
}
