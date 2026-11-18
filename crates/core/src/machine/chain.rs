//! The machine side of handoff chains (U12).

use super::Machine;
use crate::delegate::{Caller, Request};
use crate::team_tools::AskWorker;

impl Machine {
    /// `delegate_task`'s request with its `then`. U12 starts the chain; until then one with a
    /// `then` is refused.
    pub(super) fn chain_request(&self, _caller: &Caller, request: Request) -> Result<Request, String> {
        crate::chain::admit(&request)?;
        Ok(request)
    }

    /// Hook: the worker thread of task `task` settled its task. U12 hands the work to the chain's
    /// next profile, or tells the Captain once at the end.
    pub(super) fn chain_settled(&self, _task: &str) {}

    pub(super) fn ask_worker(&self, _tab: Option<&str>, _task: Option<&str>, _call: AskWorker) -> Result<String, String> {
        super::team::not_yet("ask_worker")
    }
}
