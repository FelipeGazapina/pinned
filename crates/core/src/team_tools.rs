//! The agent team's MCP tools, declared in one place: their arguments (`TeamTool`), their schemas
//! (`definition`) and who is offered each (`offered`). The MCP server lists `offered`'s tools and
//! sends a call of any of them to the machine as `agent::Message::Team`, which
//! `Machine::team_tool` (`machine/team.rs`) hands to the tool's own module.
//!
//! The room tools are offered. Each other unit turns its own tools on in its own `*_offered`
//! function below, and fills in its machine side:
//! - `task_fix`: U9 (`machine/send_back.rs`)
//! - `feature_map_read`, `feature_map_set`: U3 (`machine/feature_map.rs`)
//! - `profile_propose`: U5 (`machine/profiles.rs`)
//! - `board_note`, `board_read`: U11 (`machine/board.rs`)
//! - `ask_worker`: U12 (`machine/chain.rs`)
//! - `room_open`, `room_post`, `room_read`, `room_done`, `room_close`: `machine/room.rs`

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One call of a team tool, tagged by the tool's name.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "name", rename_all = "snake_case")]
pub enum TeamTool {
    TaskFix(TaskFix),
    FeatureMapRead(FeatureMapRead),
    FeatureMapSet(FeatureMapSet),
    ProfilePropose(ProfilePropose),
    BoardNote(BoardNoteArgs),
    BoardRead(BoardRead),
    AskWorker(AskWorker),
    RoomOpen(RoomOpen),
    RoomPost(RoomPost),
    RoomRead(RoomRead),
    RoomDone(RoomDone),
    RoomClose(RoomClose),
}

/// `task_fix`: the Captain sends a finished task's review findings back to its worker for one
/// more round, outside a handoff chain.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct TaskFix {
    #[serde(rename = "taskId")]
    pub task: String,
    /// What the Captain adds to the findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `feature_map_read`: the project's features, or only `feature`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct FeatureMapRead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
}

/// `feature_map_set`: the files of one feature, and what it does.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct FeatureMapSet {
    pub feature: String,
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
}

/// `profile_propose`: an orchestrator proposes a worker profile, which waits in
/// `WorkerProfiles::proposed` for the user.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct ProfilePropose {
    pub profile: crate::profiles::WorkerProfile,
    /// Why, in a line the user reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// `board_note`: a worker's note on its Captain's board, about one file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct BoardNoteArgs {
    pub file: String,
    /// At most `board::NOTE_CHARS` characters.
    pub text: String,
}

/// `board_read`: the board, or one task's part of it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct BoardRead {
    #[serde(default, rename = "taskId", skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// `ask_worker`: a question to another worker of the same chain, which it answers in its thread.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct AskWorker {
    #[serde(rename = "taskId")]
    pub task: String,
    pub question: String,
}

/// One worker a `room_open` seats. The name is how the others address them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct RoomSeatSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<String>,
}

/// `room_open`: the Captain opens one room and seats workers who must read each other.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct RoomOpen {
    pub brief: String,
    pub seats: Vec<RoomSeatSpec>,
    #[serde(default, rename = "postCap", skip_serializing_if = "Option::is_none")]
    pub post_cap: Option<u32>,
}

/// `room_post`: say something in the room. `to` names seats, or `captain`. Empty wakes every other seat.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct RoomPost {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    /// The caller's own id for this post. A repeat under it appends nothing and answers the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// `room_read`: the room since `since`, or the whole thread.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default, schemars::JsonSchema)]
pub struct RoomRead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
}

/// `room_done`: a seat finishes with a short summary.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct RoomDone {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
}

/// `room_close`: the Captain closes the room. Seats still running are stopped.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default, schemars::JsonSchema)]
pub struct RoomClose {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
}

impl TeamTool {
    pub fn name(&self) -> &'static str {
        match self {
            TeamTool::TaskFix(_) => "task_fix",
            TeamTool::FeatureMapRead(_) => "feature_map_read",
            TeamTool::FeatureMapSet(_) => "feature_map_set",
            TeamTool::ProfilePropose(_) => "profile_propose",
            TeamTool::BoardNote(_) => "board_note",
            TeamTool::BoardRead(_) => "board_read",
            TeamTool::AskWorker(_) => "ask_worker",
            TeamTool::RoomOpen(_) => "room_open",
            TeamTool::RoomPost(_) => "room_post",
            TeamTool::RoomRead(_) => "room_read",
            TeamTool::RoomDone(_) => "room_done",
            TeamTool::RoomClose(_) => "room_close",
        }
    }
}

/// The session a tool list is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TeamSeat {
    /// A project's Captain.
    pub captain: bool,
    /// A worker thread, or a delegated headless child.
    pub worker: bool,
    /// A delegated headless child.
    pub child: bool,
}

/// Every team tool's name, in the order `offered` lists them.
pub const NAMES: [&str; 12] = [
    "task_fix",
    "feature_map_read",
    "feature_map_set",
    "profile_propose",
    "board_note",
    "board_read",
    "ask_worker",
    "room_open",
    "room_post",
    "room_read",
    "room_done",
    "room_close",
];

pub fn named(name: &str) -> bool {
    NAMES.contains(&name)
}

/// The call `name` makes with `args`, as the MCP server reads it.
pub fn request(name: &str, args: &Value) -> Result<TeamTool, String> {
    let mut tool = if args.is_object() { args.clone() } else { json!({}) };
    tool["name"] = name.into();
    serde_json::from_value(tool).map_err(|e| format!("{name}: {e}"))
}

/// The tools `seat` is offered, as the MCP server lists them.
pub fn offered(seat: &TeamSeat) -> Vec<Value> {
    let offers: [(&str, fn(&TeamSeat) -> bool); 12] = [
        ("task_fix", task_fix_offered),
        ("feature_map_read", feature_map_read_offered),
        ("feature_map_set", feature_map_set_offered),
        ("profile_propose", profile_propose_offered),
        ("board_note", board_note_offered),
        ("board_read", board_read_offered),
        ("ask_worker", ask_worker_offered),
        ("room_open", room_open_offered),
        ("room_post", room_post_offered),
        ("room_read", room_read_offered),
        ("room_done", room_done_offered),
        ("room_close", room_close_offered),
    ];
    offers.into_iter().filter(|(_, offered)| offered(seat)).filter_map(|(name, _)| definition(name)).collect()
}

/// U9 turns `task_fix` on.
fn task_fix_offered(_seat: &TeamSeat) -> bool {
    false
}

/// U3 turns `feature_map_read` on.
fn feature_map_read_offered(_seat: &TeamSeat) -> bool {
    false
}

/// U3 turns `feature_map_set` on.
fn feature_map_set_offered(_seat: &TeamSeat) -> bool {
    false
}

/// U5 turns `profile_propose` on.
fn profile_propose_offered(_seat: &TeamSeat) -> bool {
    false
}

/// U11 turns `board_note` on.
fn board_note_offered(_seat: &TeamSeat) -> bool {
    false
}

/// U11 turns `board_read` on.
fn board_read_offered(_seat: &TeamSeat) -> bool {
    false
}

/// U12 turns `ask_worker` on.
fn ask_worker_offered(_seat: &TeamSeat) -> bool {
    false
}

/// A project's Captain opens and closes a room. A delegated child does not.
fn room_open_offered(seat: &TeamSeat) -> bool {
    seat.captain && !seat.child
}

/// The Captain and a seated worker speak and read. A plain thread does not.
fn room_post_offered(seat: &TeamSeat) -> bool {
    (seat.captain && !seat.child) || seat.worker
}

fn room_read_offered(seat: &TeamSeat) -> bool {
    room_post_offered(seat)
}

/// A seat finishes. The Captain closes the room instead.
fn room_done_offered(seat: &TeamSeat) -> bool {
    seat.worker && !seat.captain
}

fn room_close_offered(seat: &TeamSeat) -> bool {
    room_open_offered(seat)
}

/// A team tool's MCP definition. Each unit writes its own tool's descriptions when it turns it on.
pub fn definition(name: &str) -> Option<Value> {
    let task_id = json!({ "type": "string", "description": "The task's id, as delegate_task answered it." });
    let (description, properties, required): (&str, Value, &[&str]) = match name {
        "task_fix" => ("Send a finished task's review findings back to its worker for one more round.", json!({ "taskId": task_id, "note": { "type": "string" } }), &["taskId"]),
        "feature_map_read" => ("Read the project's feature map, or one feature.", json!({ "feature": { "type": "string" } }), &[]),
        "feature_map_set" => (
            "Set the files of one feature of the project's feature map.",
            json!({ "feature": { "type": "string" }, "files": { "type": "array", "items": { "type": "string" } }, "about": { "type": "string" } }),
            &["feature", "files"],
        ),
        "profile_propose" => ("Propose a worker profile for the user to take or drop.", json!({ "profile": { "type": "object" }, "why": { "type": "string" } }), &["profile"]),
        "board_note" => ("Leave a note about one file on your Captain's board.", json!({ "file": { "type": "string" }, "text": { "type": "string", "maxLength": crate::board::NOTE_CHARS } }), &["file", "text"]),
        "board_read" => ("Read your Captain's board, or one task's part of it.", json!({ "taskId": task_id }), &[]),
        "ask_worker" => ("Ask another worker of your chain a question.", json!({ "taskId": task_id, "question": { "type": "string" } }), &["taskId", "question"]),
        "room_open" => (
            "Open one room and seat workers who must read each other while they work. Each seat counts as a worker. If the last seat does not fit, nothing opens.",
            json!({
                "brief": { "type": "string" },
                "seats": { "type": "array", "items": { "type": "object", "properties": {
                    "name": { "type": "string" },
                    "model": { "type": "string" },
                    "effort": { "type": "string" },
                    "tools": { "type": "array", "items": { "type": "string" } },
                    "isolation": { "type": "string", "enum": ["shared", "worktree"] }
                }, "required": ["name"] } },
                "postCap": { "type": "integer", "minimum": 1, "maximum": 200 }
            }),
            &["brief", "seats"],
        ),
        "room_post" => (
            "Say something in the room. A reply that stays only in this session is invisible. End your turn after posting. to names seats, or captain. A retry with the same id posts nothing new.",
            json!({ "text": { "type": "string" }, "to": { "type": "array", "items": { "type": "string" } }, "room": { "type": "string" }, "id": { "type": "string" } }),
            &["text"],
        ),
        "room_read" => ("Read the room since a seq, or the whole thread.", json!({ "since": { "type": "integer" }, "room": { "type": "string" } }), &[]),
        "room_done" => ("Finish your seat with a short summary.", json!({ "summary": { "type": "string" }, "room": { "type": "string" } }), &["summary"]),
        "room_close" => ("Close the room. Seats still running are stopped.", json!({ "reason": { "type": "string" }, "room": { "type": "string" } }), &[]),
        _ => return None,
    };
    Some(json!({ "name": name, "description": description, "inputSchema": { "type": "object", "properties": properties, "required": required } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered_names(seat: TeamSeat) -> Vec<String> {
        offered(&seat).into_iter().map(|tool| tool["name"].as_str().unwrap().to_owned()).collect()
    }

    #[test]
    fn only_the_room_tools_are_offered() {
        let names = |list: &[&str]| list.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
        assert_eq!(offered_names(TeamSeat::default()), Vec::<String>::new());
        assert_eq!(offered_names(TeamSeat { captain: true, child: true, ..TeamSeat::default() }), Vec::<String>::new());
        assert_eq!(offered_names(TeamSeat { captain: true, ..TeamSeat::default() }), names(&["room_open", "room_post", "room_read", "room_close"]));
        assert_eq!(offered_names(TeamSeat { worker: true, ..TeamSeat::default() }), names(&["room_post", "room_read", "room_done"]));
        assert_eq!(offered_names(TeamSeat { worker: true, child: true, ..TeamSeat::default() }), names(&["room_post", "room_read", "room_done"]));
        let captain = offered_names(TeamSeat { captain: true, ..TeamSeat::default() });
        for name in ["task_fix", "feature_map_read", "feature_map_set", "profile_propose", "board_note", "board_read", "ask_worker"] {
            assert!(!captain.iter().any(|offered| offered == name), "{name} is offered");
        }
    }

    #[test]
    fn every_team_tool_has_a_definition_and_parses_by_its_name() {
        for name in NAMES {
            assert_eq!(definition(name).unwrap()["name"], name);
            assert!(named(name));
        }
        assert_eq!(definition("task_cancel"), None);
        let calls = [
            (json!({ "taskId": "t1" }), "task_fix"),
            (json!({}), "feature_map_read"),
            (json!({ "feature": "login", "files": ["src/login.rs"] }), "feature_map_set"),
            (json!({ "profile": { "name": "scout" } }), "profile_propose"),
            (json!({ "file": "src/a.rs", "text": "moved" }), "board_note"),
            (json!({ "taskId": "t1" }), "board_read"),
            (json!({ "taskId": "t1", "question": "which branch?" }), "ask_worker"),
            (json!({ "brief": "Fix it.", "seats": [{ "name": "Ana" }, { "name": "Beto" }] }), "room_open"),
            (json!({ "text": "hi" }), "room_post"),
            (json!({}), "room_read"),
            (json!({ "summary": "done" }), "room_done"),
            (json!({}), "room_close"),
        ];
        for (args, name) in calls {
            let tool = request(name, &args).unwrap();
            assert_eq!(tool.name(), name);
            let back = serde_json::to_value(&tool).unwrap();
            assert_eq!(back["name"], name);
            assert_eq!(serde_json::from_value::<TeamTool>(back).unwrap(), tool, "{name} round-trips");
        }
        assert_eq!(request("task_fix", &json!({})).unwrap_err(), "task_fix: missing field `taskId`");
    }
}
