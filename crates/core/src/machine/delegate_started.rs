//! The hook after a delegated task exists, for the units that keep something on its record.

use super::Machine;
use crate::delegate::Request;

impl Machine {
    /// Hook: `request` started task `task`, from `start_delegate` or, for a worker another machine
    /// sent, `start_for_origin`. A replayed request calls it again with the task it started, so what
    /// runs here sets a value and never adds one. U6 calls `Delegates::set_profile` here with the
    /// profile `profile_request` resolved, and U12 `Delegates::set_chain` with the request's `then`.
    pub(super) fn delegate_started(&self, _task: &str, _request: &Request) {}
}
