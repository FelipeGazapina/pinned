//! The machine side of handoff chains (U12).

use super::Machine;
use crate::team_tools::AskWorker;

impl Machine {
    pub(super) fn ask_worker(&self, _tab: Option<&str>, _task: Option<&str>, _call: AskWorker) -> Result<String, String> {
        super::team::not_yet("ask_worker")
    }
}
