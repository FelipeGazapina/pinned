//! The team board (U11): what a Captain's workers did, noted, handed on, are blocked on and spent,
//! read with `Call::Board`, kept apart from `Call::Plan`. Workers write notes with `board_note`
//! and read it with `board_read`; pinned writes the rest through `Machine::board_write`.
//!
//! Owner: U11 (this file and `machine/board.rs`). U13 draws it in `ui/gui-board.js`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The longest `BoardNote::text`, in characters.
pub const NOTE_CHARS: usize = 200;

/// The longest `BoardNote::file`, in characters.
pub const FILE_CHARS: usize = 300;

/// The most notes, hand-offs and blocks a board keeps, each. A new one drops the oldest.
pub const KEPT: usize = 200;

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

/// The notes, hand-offs and blocks written to each Captain's board, by the Captain's tab, oldest
/// first. Kept in memory only. Progress, spend and the tasks that wait on the user are read from
/// the task records instead.
#[derive(Debug, Default)]
pub struct Boards {
    kept: HashMap<String, BoardView>,
}

impl Boards {
    /// Adds `event` to `captain`'s board. `Settled` adds nothing: progress reads the task record.
    pub fn add(&mut self, captain: &str, event: BoardEvent) {
        match event {
            BoardEvent::Settled { .. } => {}
            BoardEvent::Note(note) => keep(&mut self.board(captain).notes, note),
            BoardEvent::Handoff(handoff) => keep(&mut self.board(captain).handoffs, handoff),
            BoardEvent::Block(block) => keep(&mut self.board(captain).blocks, block),
        }
    }

    /// What `captain`'s board keeps, with no progress or spend.
    pub fn kept(&self, captain: &str) -> BoardView {
        self.kept.get(captain).cloned().unwrap_or_default()
    }

    fn board(&mut self, captain: &str) -> &mut BoardView {
        self.kept.entry(captain.to_owned()).or_default()
    }
}

fn keep<T>(list: &mut Vec<T>, entry: T) {
    if list.len() >= KEPT {
        list.remove(0);
    }
    list.push(entry);
}

/// `board_note`'s file and text, trimmed, or why the note is refused. Nothing is cut.
pub fn check_note(file: &str, text: &str) -> Result<(String, String), String> {
    let (file, text) = (file.trim(), text.trim());
    if file.is_empty() {
        return Err("a note needs the file it is about".into());
    }
    if file.chars().count() > FILE_CHARS {
        return Err(format!("a note's file is at most {FILE_CHARS} characters"));
    }
    if text.is_empty() {
        return Err("a note needs a text".into());
    }
    if text.chars().count() > NOTE_CHARS {
        return Err(format!("a note is at most {NOTE_CHARS} characters: shorten it"));
    }
    Ok((file.to_owned(), text.to_owned()))
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

    #[test]
    fn a_note_is_trimmed_and_refused_empty_or_too_long_but_never_cut() {
        assert_eq!(check_note("  src/a.rs ", " moved \n"), Ok(("src/a.rs".into(), "moved".into())));
        assert!(check_note("", "moved").is_err());
        assert!(check_note("   ", "moved").is_err());
        assert!(check_note("src/a.rs", "").is_err());
        assert!(check_note("src/a.rs", " \t ").is_err());
        let long = check_note("src/a.rs", &"x".repeat(NOTE_CHARS + 1)).unwrap_err();
        assert!(long.contains("200"), "{long}");
        assert!(check_note(&"a".repeat(FILE_CHARS + 1), "moved").is_err());
        assert!(check_note(&"a".repeat(FILE_CHARS), "moved").is_ok());
        let accents = "é".repeat(NOTE_CHARS);
        assert_eq!(check_note("src/a.rs", &accents), Ok(("src/a.rs".into(), accents.clone())), "200 characters of two bytes each fit");
    }

    #[test]
    fn a_full_list_drops_its_oldest_and_settled_keeps_nothing() {
        let mut boards = Boards::default();
        for at in 1..=KEPT as u64 + 1 {
            boards.add("tab-1", BoardEvent::Note(BoardNote { by: "t1".into(), file: "src/a.rs".into(), text: format!("note {at}"), at }));
        }
        let kept = boards.kept("tab-1");
        assert_eq!(kept.notes.len(), 200);
        assert_eq!(kept.notes[0].text, "note 2", "the first note is gone");
        assert_eq!(kept.notes[199].text, "note 201", "the newest is last");
        for at in 1..=KEPT as u64 + 1 {
            boards.add("tab-1", BoardEvent::Handoff(BoardHandoff { from: "t1".into(), to: "t2".into(), at }));
            boards.add("tab-1", BoardEvent::Block(BoardBlock { task: "t2".into(), why: "waits on t1".into(), at }));
        }
        let kept = boards.kept("tab-1");
        assert_eq!((kept.handoffs.len(), kept.handoffs[0].at), (200, 2));
        assert_eq!((kept.blocks.len(), kept.blocks[0].at), (200, 2));
        boards.add("tab-2", BoardEvent::Settled { task: "t3".into() });
        assert_eq!(boards.kept("tab-2"), BoardView::default());
        assert_eq!(boards.kept("tab-1").notes, kept.notes, "another board is apart");
    }
}
