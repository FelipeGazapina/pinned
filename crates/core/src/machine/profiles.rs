//! The machine side of worker profiles: U5's signed-out store and `profile_propose`, and U6's
//! `delegate_task` with `profile`.

use super::Machine;
use crate::delegate::{Caller, Request};
use crate::profiles::WorkerProfile;
use crate::taste::{self, Tastes};
use crate::team_tools::ProfilePropose;

impl Machine {
    /// `Tastes::set_worker_profiles` on this machine's own tastes, for a signed-out app.
    pub fn set_local_worker_profiles(&self, project: Option<&str>, profiles: Vec<WorkerProfile>) -> Result<Tastes, String> {
        taste::change(self.orchestration_dir.as_deref(), |tastes| tastes.set_worker_profiles(project, profiles))
    }

    /// `delegate_task`'s request as the profile it names sets it up. U6 resolves the profile;
    /// until then one that names a profile is refused.
    pub(super) fn profile_request(&self, _caller: &Caller, request: Request) -> Result<Request, String> {
        crate::profiles::admit(&request)?;
        Ok(request)
    }

    pub(super) fn profile_propose(&self, _tab: Option<&str>, _task: Option<&str>, _call: ProfilePropose) -> Result<String, String> {
        super::team::not_yet("profile_propose")
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
}
