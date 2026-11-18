//! The machine side of worker profiles: U5's signed-out store and `profile_propose`, and U6's
//! `delegate_task` with `profile`.

use super::Machine;
use crate::delegate::{Caller, Request};
use crate::global_orchestrator as global;
use crate::profiles::WorkerProfile;
use crate::taste::{self, Tastes};
use crate::team_tools::ProfilePropose;

impl Machine {
    /// `Tastes::set_worker_profiles` on this machine's own tastes, for a signed-out app, then
    /// `Tastes::drop_proposed` in the same write, as the hub's `SetWorkerProfiles` does.
    pub fn set_local_worker_profiles(&self, project: Option<&str>, profiles: Vec<WorkerProfile>, drop_proposed: &[String]) -> Result<Tastes, String> {
        self.check_skills(&profiles)?;
        taste::change(self.orchestration_dir.as_deref(), |tastes| {
            tastes.set_worker_profiles(project, profiles)?;
            tastes.drop_proposed(drop_proposed);
            Ok(())
        })
    }

    /// Whether every skill `profiles` name is installed here, as the `skills` action lists them.
    fn check_skills(&self, profiles: &[WorkerProfile]) -> Result<(), String> {
        if profiles.iter().all(|p| p.skills.is_empty()) {
            return Ok(());
        }
        let listing = crate::skills::list(&self.places());
        let installed: Vec<&str> = listing.copies.iter().map(|copy| copy.key.as_str()).collect();
        profiles.iter().try_for_each(|p| p.check_skills(&installed))
    }

    /// `delegate_task`'s request as the profile it names sets it up. U6 resolves the profile;
    /// until then one that names a profile is refused.
    pub(super) fn profile_request(&self, _caller: &Caller, request: Request) -> Result<Request, String> {
        crate::profiles::admit(&request)?;
        Ok(request)
    }

    /// `profile_propose`, the global orchestrator's alone: keeps the profile in `proposed`, which
    /// `delegate_task` never reads, until the user takes it into a list. `why` is not kept.
    pub(super) fn profile_propose(&self, tab: Option<&str>, task: Option<&str>, call: ProfilePropose) -> Result<String, String> {
        let orchestrator = task.is_none() && tab.is_some_and(|tab| self.tab(tab, |t| global::is_global(&t.env)).unwrap_or(false));
        if !orchestrator {
            return Err("profile_propose is the Orchestrator's own: a Captain or a worker cannot propose a profile".into());
        }
        self.check_skills(std::slice::from_ref(&call.profile))?;
        let name = call.profile.name.trim().to_owned();
        // The role is checked here, before the hub or this machine's tastes see anything: the hub
        // also takes a Captain's, and the signed-out path checks no role at all.
        self.propose_worker_profile(tab.unwrap_or_default(), call.profile)?;
        Ok(format!("{name} waits for the user on the Way of working page. delegate_task cannot name it until they take it into a list."))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::launch;
    use super::*;
    use crate::accounts::{Account, Provider};
    use crate::agent::{role_env, Role};
    use crate::profiles::{ProfileAccess, ProfileAuthor};

    fn proposal(name: &str) -> ProfilePropose {
        let profile = WorkerProfile { name: name.into(), skills: vec![], tools: vec!["Read".into()], access: ProfileAccess::ReadOnly, model: None, effort: None, delivers: "A map of the code.".into(), by: ProfileAuthor::User };
        ProfilePropose { profile, why: Some("Every task starts with a read.".into()) }
    }

    #[test]
    fn only_the_orchestrator_may_propose_a_profile_and_it_waits_unapproved() {
        let dir = tempfile::tempdir().unwrap();
        let m = Machine::new(launch(), Some(dir.path().into()), None, |_| {});
        let tab = |env| m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], env, false, None);
        let (captain, worker, plain, global) = (tab(role_env(Role::Orchestrator, "/repo".as_ref())), tab(role_env(Role::Worker, "/repo".as_ref())), tab(vec![]), tab(crate::global_orchestrator::env()));
        let refused = "profile_propose is the Orchestrator's own: a Captain or a worker cannot propose a profile";
        for (tab, task) in [(Some(captain.as_str()), None), (Some(worker.as_str()), None), (Some(plain.as_str()), None), (None, Some("t1")), (Some(global.as_str()), Some("t1")), (None, None)] {
            assert_eq!(m.profile_propose(tab, task, proposal("scout")), Err(refused.into()), "{tab:?} {task:?}");
        }
        assert_eq!(m.local_tastes().worker_profiles, Default::default(), "a refused proposal wrote nothing");

        let answer = m.profile_propose(Some(&global), None, proposal("scout")).unwrap();
        assert!(answer.contains("waits for the user"), "{answer}");
        let tastes = m.local_tastes();
        assert_eq!(tastes.worker_profiles.proposed.iter().map(|p| (p.name.as_str(), p.by)).collect::<Vec<_>>(), [("scout", ProfileAuthor::Orchestrator)]);
        assert!(tastes.worker_profiles.account.is_empty() && tastes.worker_profiles.projects.is_empty(), "only proposed changed");
        assert_eq!(crate::profiles::resolve(&tastes, None, "scout"), Err(crate::profiles::Unresolved::NotApproved("scout".into())));
    }

    #[test]
    fn a_signed_out_approve_or_discard_drops_the_proposal_in_the_same_write() {
        let dir = tempfile::tempdir().unwrap();
        let m = Machine::new(launch(), Some(dir.path().into()), None, |_| {});
        let global = m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], crate::global_orchestrator::env(), false, None);
        for name in ["scout", "fixer"] {
            m.profile_propose(Some(&global), None, proposal(name)).unwrap();
        }
        let approved = m.set_local_worker_profiles(None, vec![proposal("Scout").profile], &["scout".into()]).unwrap();
        assert_eq!(approved.worker_profiles.account.iter().map(|p| (p.name.as_str(), p.by)).collect::<Vec<_>>(), [("Scout", ProfileAuthor::User)]);
        assert_eq!(approved.worker_profiles.proposed.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["fixer"]);
        let discarded = m.set_local_worker_profiles(None, approved.worker_profiles.account.clone(), &["fixer".into()]).unwrap();
        assert_eq!((discarded.worker_profiles.account, discarded.worker_profiles.proposed), (approved.worker_profiles.account, vec![]));
        let before = m.local_tastes();
        assert!(m.set_local_worker_profiles(None, vec![proposal(" ").profile], &["scout".into()]).is_err());
        assert_eq!(m.local_tastes(), before, "a refused list drops nothing");
    }

    #[test]
    fn a_linked_machine_sends_the_orchestrators_proposal_to_the_hub_and_nobody_elses() {
        use std::sync::{Arc, Mutex};
        let dir = tempfile::tempdir().unwrap();
        let m = Machine::new(launch(), Some(dir.path().into()), None, |_| {});
        let sent: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
        let seen = sent.clone();
        m.set_hub(Arc::new(move |call| {
            seen.lock().unwrap().push(serde_json::to_value(&call).unwrap());
            let mut tastes = Tastes::default();
            if let crate::wire::HubCall::ProposeWorkerProfile { profile, .. } = call {
                tastes.propose_worker_profile(profile)?;
            }
            Ok(serde_json::to_value(tastes).unwrap())
        }));
        let tab = |env| m.add_tab("/repo".into(), Account::builtin(Provider::Claude), vec![], env, false, None);
        let (captain, worker, plain, global) = (tab(role_env(Role::Orchestrator, "/repo".as_ref())), tab(role_env(Role::Worker, "/repo".as_ref())), tab(vec![]), tab(crate::global_orchestrator::env()));
        let refused = "profile_propose is the Orchestrator's own: a Captain or a worker cannot propose a profile";
        for (tab, task) in [(Some(captain.as_str()), None), (Some(worker.as_str()), None), (Some(plain.as_str()), None), (None, Some("t1")), (Some(global.as_str()), Some("t1"))] {
            assert_eq!(m.profile_propose(tab, task, proposal("scout")), Err(refused.into()), "{tab:?} {task:?}");
        }
        assert!(sent.lock().unwrap().is_empty(), "a refused proposal sent nothing");

        let answer = m.profile_propose(Some(&global), None, proposal(" scout ")).unwrap();
        assert!(answer.contains("scout waits for the user"), "{answer}");
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!((&sent[0]["cmd"], &sent[0]["tab"], &sent[0]["profile"]["name"]), (&serde_json::json!("propose_worker_profile"), &serde_json::json!(global), &serde_json::json!(" scout ")));
        assert_eq!(m.local_tastes().worker_profiles, Default::default(), "the hub keeps a linked machine's proposal, not this machine");
    }
}
