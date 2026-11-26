//! Pre-authorized approvals (#159). Slice 1a: the never list, which the team box hook and the
//! worker guard (`worker_guard`, #538) read. A manage-only Captain's guard (`captain_guard`) reads
//! its shell lines with the same parser.

pub mod captain_guard;
pub mod never;
pub mod team_hook;
pub mod worker_guard;
