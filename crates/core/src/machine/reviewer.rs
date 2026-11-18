//! The machine side of the Reviewer: U4's signed-out store and U8's review run. U9's send-back
//! is in `send_back.rs`.

use super::Machine;
use crate::reviewer::ReviewerPatch;
use crate::taste::{self, Tastes};

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
}

#[cfg(test)]
mod tests {
    use super::super::tests::launch;
    use super::*;

    #[test]
    fn nothing_reviews_yet() {
        let m = Machine::new(launch(), None, None, |_| {});
        assert!(!m.review_turn_end("t1", "tab-1"));
    }
}
