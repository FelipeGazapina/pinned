//! The team board (U11): what a Captain's workers did, noted, handed on, are blocked on and spent,
//! read with `Call::Board`, kept apart from `Call::Plan`. Workers write notes with `board_note`
//! and read it with `board_read`; pinned writes the rest through `Machine::board_write`.
//!
//! Owner: U11 (this file and `machine/board.rs`). U13 draws it in `ui/gui-board.js`.

use serde::{Deserialize, Serialize};

/// The longest `BoardNote::text`, in characters.
pub const NOTE_CHARS: usize = 200;

/// `Call::Board`'s answer.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct BoardView {
    pub progress: Vec<BoardProgress>,
    pub notes: Vec<BoardNote>,
    pub handoffs: Vec<BoardHandoff>,
    pub blocks: Vec<BoardBlock>,
    pub spend: BoardSpend,
}

/// Where one task of the board stands.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct BoardProgress {
    pub task: String,
    pub title: String,
    /// As `task_status`'s `workState`.
    pub state: String,
    /// Unix milliseconds of the last change.
    pub at: u64,
}

/// A note a worker left on the board.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct BoardNote {
    /// The task that wrote it.
    pub by: String,
    /// The file it is about, relative to the project.
    pub file: String,
    /// At most 200 characters.
    pub text: String,
    /// Unix milliseconds.
    pub at: u64,
}

/// One task handing its work to the next.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct BoardHandoff {
    pub from: String,
    pub to: String,
    pub at: u64,
}

/// A task that waits on another, or on the user.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct BoardBlock {
    pub task: String,
    pub why: String,
    pub at: u64,
}

/// What the board's tasks spent, in US cents.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct BoardSpend {
    pub cents: u64,
}

/// What pinned writes to a board through `Machine::board_write`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardEvent {
    /// A worker's turn ended and pinned settled its task.
    Settled { task: String },
    Note(BoardNote),
    Handoff(BoardHandoff),
    Block(BoardBlock),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_board_view_round_trips_and_reads_empty_from_nothing() {
        assert_eq!(serde_json::from_value::<BoardView>(json!({})).unwrap(), BoardView::default());
        let view = BoardView {
            progress: vec![BoardProgress { task: "t1".into(), title: "Fix login".into(), state: "working".into(), at: 1 }],
            notes: vec![BoardNote { by: "t1".into(), file: "src/login.rs".into(), text: "the token check moved".into(), at: 2 }],
            handoffs: vec![BoardHandoff { from: "t1".into(), to: "t2".into(), at: 3 }],
            blocks: vec![BoardBlock { task: "t2".into(), why: "waits on t1".into(), at: 4 }],
            spend: BoardSpend { cents: 120 },
        };
        let value = serde_json::to_value(&view).unwrap();
        assert_eq!(value["notes"][0], json!({ "by": "t1", "file": "src/login.rs", "text": "the token check moved", "at": 2 }));
        assert_eq!(serde_json::from_value::<BoardView>(value).unwrap(), view);
    }
}
