//! The machine side of the team board (U11).

use super::Machine;
use crate::board::{BoardEvent, BoardView};
use crate::team_tools::{BoardNoteArgs, BoardRead};

impl Machine {
    /// `Call::Board`: the board of the Captain of `tab`, or only `task`'s part. Empty for now.
    pub(super) fn board(&self, _tab: &str, _task: Option<&str>) -> Result<BoardView, String> {
        Ok(BoardView::default())
    }

    /// U11's hook: pinned writes `event` of task `task` to its Captain's board. Nothing keeps a
    /// board yet.
    pub(super) fn board_write(&self, _task: &str, _event: BoardEvent) {}

    pub(super) fn board_note(&self, _tab: Option<&str>, _task: Option<&str>, _call: BoardNoteArgs) -> Result<String, String> {
        super::team::not_yet("board_note")
    }

    pub(super) fn board_read(&self, _tab: Option<&str>, _task: Option<&str>, _call: BoardRead) -> Result<String, String> {
        super::team::not_yet("board_read")
    }
}
