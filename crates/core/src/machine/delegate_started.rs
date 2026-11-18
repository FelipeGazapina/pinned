//! The hook after a delegated task exists, for the units that keep something on its record.

use super::Machine;
use crate::delegate::Request;

impl Machine {
    /// Hook: `request` started task `task`. It runs once, when the task's record is new
    /// (`Claim::Fresh` in `launch_child`, and in `delegate_elsewhere` for a task another machine
    /// runs) and before its worker is spawned, so a worker that ends at once ends with what this
    /// writes. A replayed request answers the task it started and never gets here, so a retry with
    /// another `profile` or `then` changes nothing on it. U6 calls `Delegates::set_profile` here
    /// with the profile `profile_request` resolved, and U12 `Delegates::set_chain` with the
    /// request's `then`.
    pub(super) fn delegate_started(&self, task: &str, request: &Request) {
        self.delegates.set_profile(task, request.profile.clone());
    }
}
