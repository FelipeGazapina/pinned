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
