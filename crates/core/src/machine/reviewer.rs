//! The machine side of the Reviewer: U4's signed-out store, U8's review run and U9's send-back.

use super::Machine;
use crate::reviewer::{ReviewerPatch, SendBackBy};
use crate::taste::{self, Tastes};
use crate::team_tools::TaskFix;

impl Machine {
    /// `Tastes::set_reviewer` on this machine's own tastes, for a signed-out app.
    pub fn set_local_reviewer(&self, patch: ReviewerPatch) -> Result<Tastes, String> {
        taste::change(self.orchestration_dir.as_deref(), |tastes| tastes.set_reviewer(patch))
    }

    /// U8's hook: the worker thread in `tab` running task `task` finished a turn that the task
    /// would end with. True when a review now holds the task's end; the worker's notice then waits
    /// for the review. Nothing reviews yet.
    pub(super) fn review_turn_end(&self, _task: &str, _tab: &str) -> bool {
        false
    }

    /// U9's: sends round `round`'s findings of task `task` back to its worker. The Captain sends
    /// them with `task_fix`, and a handoff chain on its own (U12).
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
    fn nothing_reviews_or_sends_back_yet() {
        let m = Machine::new(launch(), None, None, |_| {});
        assert!(!m.review_turn_end("t1", "tab-1"));
        for by in [SendBackBy::Captain, SendBackBy::Chain] {
            assert_eq!(m.send_back("t1", 1, by), Err("sending review findings back is not available in this build".into()));
        }
    }
}
