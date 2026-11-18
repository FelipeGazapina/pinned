//! U9: sending a review's findings back to the worker, by the Captain with `task_fix` or by a
//! handoff chain on its own (U12 calls `send_back` with `SendBackBy::Chain`).

use super::Machine;
use crate::reviewer::SendBackBy;
use crate::team_tools::TaskFix;

impl Machine {
    /// Sends round `round`'s findings of task `task` back to its worker.
    pub fn send_back(&self, _task: &str, _round: u8, _by: SendBackBy) -> Result<(), String> {
        Err("sending review findings back is not available in this build".into())
    }

    pub(super) fn task_fix(&self, _tab: Option<&str>, _task: Option<&str>, _call: TaskFix) -> Result<String, String> {
        super::team::not_yet("task_fix")
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::launch;
    use super::*;

    #[test]
    fn nothing_sends_findings_back_yet() {
        let m = Machine::new(launch(), None, None, |_| {});
        for by in [SendBackBy::Captain, SendBackBy::Chain] {
            assert_eq!(m.send_back("t1", 1, by), Err("sending review findings back is not available in this build".into()));
        }
    }
}
