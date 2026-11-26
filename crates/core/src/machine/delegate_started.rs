//! The hook after a delegated task exists, for the units that keep something on its record.

use super::Machine;
use crate::delegate::Request;

impl Machine {
    /// Hook: `request` started task `task`, from `start_delegate` or, for a worker another machine
    /// sent, `start_for_origin`. A replayed request calls it again with the task it started, so what
    /// runs here sets a value and never adds one. The profile `profile_request` resolved is kept
    /// once, so a replay never changes it, and U12 adds `Delegates::set_chain` with the request's `then`.
    pub(super) fn delegate_started(&self, task: &str, request: &Request) {
        if request.profile.is_some() && self.delegates.get(task).is_some_and(|task| task.profile.is_none()) {
            self.delegates.set_profile(task, request.profile.clone());
        }
    }
}
