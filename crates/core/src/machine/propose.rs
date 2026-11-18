//! The machine's half of `HubCall::ProposeWorkerProfile`, which U5's `profile_propose` calls.

use super::Machine;
use crate::profiles::WorkerProfile;
use crate::taste::{self, Tastes};
use crate::wire::HubCall;

impl Machine {
    /// Proposes `profile` from the session in `tab` for the user to take or drop: through the hub
    /// while this machine is linked, which takes it only from a tab this machine reports as a
    /// Captain or the global orchestrator, else into this machine's own tastes. Either way it is
    /// `Tastes::propose_worker_profile`. Signed out, the caller checks that `tab` is one.
    pub fn propose_worker_profile(&self, tab: &str, profile: WorkerProfile) -> Result<Tastes, String> {
        let hub = self.hub.lock().unwrap().clone();
        match hub {
            Some(hub) => serde_json::from_value(hub(HubCall::ProposeWorkerProfile { tab: tab.into(), profile })?).map_err(|e| format!("the hub's answer did not read: {e}")),
            None => taste::change(self.orchestration_dir.as_deref(), |tastes| tastes.propose_worker_profile(profile)),
        }
    }
}
