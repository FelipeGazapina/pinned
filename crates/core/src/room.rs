//! One shared thread several workers sit in. The project's Captain opens it with `room_open`.
//! Speech, the brief and each seat's summary live here. A seat's tools, diffs and worktree stay
//! on that seat's own tab. Wakes leave through the notice queue: this module only says who to tell.
//!
//! A room may debate in rounds. Each working seat posts once a round, and a seat that has posted
//! sleeps until the next round's line. After the last round the synthesis seat posts once, and that closes the room.
//!
//! A room tab is not a Claude session. `PINNED_ROOM` on a tab with no role is that thread.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const ENV: &str = "PINNED_ROOM";
pub const FILE: &str = "rooms.json";
pub const CAPTAIN: &str = "captain";
pub const DEFAULT_POST_CAP: u32 = 40;
pub const MAX_POST_CAP: u32 = 200;
pub const MAX_SEATS: usize = 12;
pub const MAX_ROUNDS: u32 = 10;
pub const MIN_SEATS: usize = 2;
pub const TEXT_MAX: usize = 8_000;
const NAME_MAX: usize = 40;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    Settled,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "who")]
pub enum Author {
    Seat { name: String },
    Card,
    User,
    System,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub task: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tab: String,
    #[serde(default)]
    pub cursor: u64,
    #[serde(default)]
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Post {
    pub seq: u64,
    pub author: Author,
    /// Who this post wakes. Empty wakes nobody. `captain` is the owner tab.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    pub text: String,
    pub at: u64,
    /// The id its author gave it. A repeat under the same id answers this post.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Room {
    pub id: String,
    pub project: String,
    pub owner_tab: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tab: String,
    pub brief: String,
    pub status: Status,
    pub seq: u64,
    #[serde(default = "default_cap")]
    pub post_cap: u32,
    pub seats: Vec<Seat>,
    #[serde(default)]
    pub posts: Vec<Post>,
    /// Last seq included in a notice queued for the Captain.
    #[serde(default)]
    pub owner_cursor: u64,
    pub opened: u64,
    /// Set when the room debates in rounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounds: Option<Rounds>,
    /// False until every seat has a task. Absent on disk, so a loaded room is published.
    #[serde(default = "published_yes", skip_serializing)]
    pub published: bool,
}

/// A debate in rounds, then the synthesis seat's one post.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Rounds {
    pub total: u32,
    /// From 1. It stays at `total` while the room waits for the synthesis.
    pub round: u32,
    /// The seats that have posted in `round`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub posted: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthesis: Option<String>,
    /// The rounds are over, and only the synthesis seat posts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub synthesizing: bool,
}

fn default_cap() -> u32 {
    DEFAULT_POST_CAP
}

fn published_yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    Seat(String),
    Captain,
}

impl Who {
    pub fn key(&self) -> &str {
        match self {
            Who::Seat(name) => name,
            Who::Captain => CAPTAIN,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wake {
    pub key: String,
    pub tab: String,
    pub who: Who,
    pub through: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub wakes: Vec<Wake>,
    pub closed: bool,
    pub seq: u64,
    /// The rounds or the synthesis closed the room, so every seat task still running stops.
    pub rounds_closed: bool,
}

/// What `room_post` says, before the room resolves who it wakes.
#[derive(Clone, Debug)]
pub enum Speech {
    Seat { name: String, text: String, to: Vec<String> },
    Card { text: String, to: Vec<String> },
    User { text: String },
}

impl Speech {
    fn author(&self) -> Author {
        match self {
            Speech::Seat { name, .. } => Author::Seat { name: name.clone() },
            Speech::Card { .. } => Author::Card,
            Speech::User { .. } => Author::User,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    next: u64,
    #[serde(default)]
    rooms: Vec<Room>,
}

pub struct Store {
    path: Option<PathBuf>,
    rooms: Vec<Room>,
    next: u64,
}

impl Store {
    pub fn load(dir: Option<&Path>) -> Self {
        let path = dir.map(|dir| dir.join(FILE));
        let mut saved = Saved::default();
        if let Some(path) = &path {
            match std::fs::read(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => keep_aside(path, &e.to_string()),
                Ok(bytes) => match serde_json::from_slice::<Saved>(&bytes) {
                    Ok(read) => saved = read,
                    Err(e) => keep_aside(path, &format!("no JSON rooms at line {} column {}", e.line(), e.column())),
                },
            }
        }
        Store { path, rooms: saved.rooms, next: saved.next }
    }

    pub fn get(&self, id: &str) -> Option<&Room> {
        self.rooms.iter().find(|room| room.id == id)
    }

    pub fn published_ids(&self) -> Vec<String> {
        self.rooms.iter().filter(|room| room.published).map(|room| room.id.clone()).collect()
    }

    /// The room id on a tab that carries `PINNED_ROOM` and no role.
    pub fn id_of_tab(&self, tab: &str) -> Option<String> {
        self.rooms.iter().find(|room| room.tab == tab).map(|room| room.id.clone())
    }

    /// Starts a room in memory and consumes its id. It is not on disk until `publish`. With `rounds`
    /// the seats debate in that many rounds, and `synthesis` names the seat that writes the synthesis.
    #[allow(clippy::too_many_arguments)]
    pub fn open(&mut self, project: &str, owner_tab: &str, brief: &str, names: &[String], post_cap: Option<u32>, rounds: Option<u32>, synthesis: Option<&str>, now: u64) -> Result<String, String> {
        let brief = check_brief(brief)?;
        let names = check_names(names)?;
        let post_cap = post_cap.map_or(DEFAULT_POST_CAP, |cap| cap.clamp(1, MAX_POST_CAP));
        let rounds = check_rounds(rounds, synthesis, &names, post_cap)?;
        self.next += 1;
        let id = format!("room-{}", self.next);
        self.rooms.push(Room {
            id: id.clone(),
            project: project.to_owned(),
            owner_tab: owner_tab.to_owned(),
            tab: String::new(),
            brief,
            status: Status::Open,
            seq: 0,
            post_cap,
            seats: names.into_iter().map(|name| Seat { name, task: String::new(), tab: String::new(), cursor: 0, done: false, summary: None }).collect(),
            posts: vec![],
            owner_cursor: 0,
            opened: now,
            rounds,
            published: false,
        });
        self.write();
        Ok(id)
    }

    pub fn set_tab(&mut self, id: &str, tab: &str) {
        if let Some(room) = self.rooms.iter_mut().find(|room| room.id == id) {
            room.tab = tab.to_owned();
        }
    }

    pub fn bind_seat(&mut self, id: &str, name: &str, task: &str, tab: &str) -> Result<(), String> {
        let room = self.room_mut(id)?;
        let seat = room.seats.iter_mut().find(|seat| seat.name == name).ok_or_else(|| format!("no seat {name} in {id}"))?;
        seat.task = task.to_owned();
        seat.tab = tab.to_owned();
        Ok(())
    }

    pub fn publish(&mut self, id: &str) -> Result<(), String> {
        self.room_mut(id)?.published = true;
        self.write();
        Ok(())
    }

    /// Drops a room that never opened, or one being rolled back. A published room leaves the file.
    pub fn discard(&mut self, id: &str) {
        let published = self.rooms.iter().any(|room| room.id == id && room.published);
        self.rooms.retain(|room| room.id != id);
        if published {
            self.write();
        }
    }

    pub fn post(&mut self, id: &str, speech: Speech, now: u64) -> Result<Outcome, String> {
        self.post_with(id, speech, None, now)
    }

    /// `post` under the author's own `post_id`. A repeat of that id appends nothing and answers the first post.
    /// In a debate a seat posts once a round, and once the rounds are over only the synthesis seat posts.
    pub fn post_with(&mut self, id: &str, speech: Speech, post_id: Option<&str>, now: u64) -> Result<Outcome, String> {
        let index = self.index(id)?;
        let post_id = post_id.map(str::trim).filter(|post_id| !post_id.is_empty());
        if let Some(post_id) = post_id {
            let room = &self.rooms[index];
            let author = speech.author();
            if let Some(first) = room.posts.iter().find(|post| post.author == author && post.post_id.as_deref() == Some(post_id)) {
                return Ok(Outcome { wakes: vec![], closed: room.status != Status::Open, seq: first.seq, rounds_closed: false });
            }
        }
        if self.rooms[index].status != Status::Open {
            return Err(format!("{id} is closed"));
        }
        let (author, text, to) = match speech {
            Speech::Seat { name, text, to } => {
                if !self.rooms[index].seats.iter().any(|seat| seat.name == name && !seat.done) {
                    return Err(match self.rooms[index].seats.iter().any(|seat| seat.name == name) {
                        true => format!("{name} already finished"),
                        false => format!("no seat {name} in {id}"),
                    });
                }
                if let Some(rounds) = &self.rooms[index].rounds {
                    check_turn(rounds, &name)?;
                }
                (Author::Seat { name: name.clone() }, check_text(&text)?, self.audience(index, Some(&name), &to, false)?)
            }
            Speech::Card { text, to } => (Author::Card, check_text(&text)?, self.audience(index, None, &to, false)?),
            Speech::User { text } => (Author::User, check_text(&text)?, self.audience(index, None, &[], true)?),
        };
        self.push(index, author.clone(), text, to, now);
        if let Some(post) = self.rooms[index].posts.last_mut() {
            post.post_id = post_id.map(str::to_owned);
        }
        let rounds_closed = match &author {
            Author::Seat { name } => self.seat_spoke(index, name, now),
            _ => false,
        };
        let closed = rounds_closed || self.close_if_full(index, now);
        let wakes = self.wakes(index);
        self.quiet_author(index, &author);
        if self.rooms[index].published {
            self.write();
        }
        Ok(Outcome { seq: self.rooms[index].seq, closed, wakes, rounds_closed })
    }

    /// A seat's `room_done`. The round no longer waits on it.
    pub fn finish(&mut self, id: &str, name: &str, summary: &str, now: u64) -> Result<Outcome, String> {
        let summary = check_text(summary).map_err(|_| "room_done needs a summary".to_owned())?;
        let index = self.index(id)?;
        if self.rooms[index].status != Status::Open {
            return Err(format!("{id} is closed"));
        }
        let seat = self.rooms[index].seats.iter_mut().find(|seat| seat.name == name).ok_or_else(|| format!("no seat {name} in {id}"))?;
        if seat.done {
            return Err(format!("{name} already finished"));
        }
        seat.done = true;
        seat.summary = Some(summary.clone());
        let audience = self.audience(index, Some(name), &[], true)?;
        self.push(index, Author::System, format!("{name} finished: {summary}"), audience, now);
        let settled = self.settle_if_all_done(index, now);
        let rounds_closed = self.end_round_if_due(index, now);
        let closed = settled || rounds_closed || self.close_if_full(index, now);
        let wakes = self.wakes(index);
        if self.rooms[index].published {
            self.write();
        }
        Ok(Outcome { seq: self.rooms[index].seq, closed, wakes, rounds_closed })
    }

    /// A seat whose task died. Marks it done so the room can still finish, and the round no longer waits on it.
    pub fn fail_seat(&mut self, id: &str, name: &str, error: &str, now: u64) -> Result<Outcome, String> {
        let index = self.index(id)?;
        if self.rooms[index].status != Status::Open {
            return Ok(Outcome { wakes: vec![], closed: true, seq: self.rooms[index].seq, rounds_closed: false });
        }
        let Some(seat) = self.rooms[index].seats.iter_mut().find(|seat| seat.name == name) else {
            return Ok(Outcome { wakes: vec![], closed: false, seq: self.rooms[index].seq, rounds_closed: false });
        };
        if seat.done {
            return Ok(Outcome { wakes: vec![], closed: false, seq: self.rooms[index].seq, rounds_closed: false });
        }
        let error = one_line(error);
        seat.done = true;
        seat.summary = Some(error.clone());
        let audience = self.audience(index, Some(name), &[], true)?;
        self.push(index, Author::System, format!("{name} failed: {error}"), audience, now);
        let settled = self.settle_if_all_done(index, now);
        let rounds_closed = self.end_round_if_due(index, now);
        let wakes = self.wakes(index);
        if self.rooms[index].published {
            self.write();
        }
        Ok(Outcome { seq: self.rooms[index].seq, closed: settled || rounds_closed, wakes, rounds_closed })
    }

    /// The seat's task is already over, so the room must not wait on it. No finish line: the task's
    /// own report already said why. When that was the last open seat, the room closes. When the
    /// round waited only on it, the round ends.
    pub fn seat_gone(&mut self, id: &str, name: &str, now: u64) -> Result<Outcome, String> {
        let index = self.index(id)?;
        if self.rooms[index].status != Status::Open {
            return Ok(Outcome { wakes: vec![], closed: true, seq: self.rooms[index].seq, rounds_closed: false });
        }
        if let Some(seat) = self.rooms[index].seats.iter_mut().find(|seat| seat.name == name) {
            seat.done = true;
        }
        let settled = self.settle_if_all_done(index, now);
        let rounds_closed = self.end_round_if_due(index, now);
        let wakes = self.wakes(index);
        if self.rooms[index].published {
            self.write();
        }
        Ok(Outcome { seq: self.rooms[index].seq, closed: settled || rounds_closed, wakes, rounds_closed })
    }

    pub fn close(&mut self, id: &str, reason: &str, now: u64) -> Result<Outcome, String> {
        let index = self.index(id)?;
        if self.rooms[index].status != Status::Open {
            return Err(format!("{id} is closed"));
        }
        self.rooms[index].status = Status::Settled;
        let reason = reason.trim();
        self.push(index, Author::System, close_text(&self.rooms[index], reason), vec![CAPTAIN.to_owned()], now);
        let wakes = self.wakes(index);
        if self.rooms[index].published {
            self.write();
        }
        Ok(Outcome { seq: self.rooms[index].seq, closed: true, wakes, rounds_closed: false })
    }

    /// The room's page. A debate adds its `round`, `rounds` and `synthesis` seat.
    pub fn read(&self, id: &str, since: u64) -> Result<Value, String> {
        let room = self.get(id).ok_or_else(|| format!("no room {id}"))?;
        let posts: Vec<Value> = room
            .posts
            .iter()
            .filter(|post| post.seq > since)
            .map(|post| json!({ "seq": post.seq, "author": label(&post.author), "text": post.text, "at": post.at }))
            .collect();
        let seats: Vec<Value> = room.seats.iter().map(|seat| json!({ "name": seat.name, "done": seat.done, "summary": seat.summary })).collect();
        let mut page = json!({ "id": room.id, "status": match room.status { Status::Open => "open", Status::Settled => "settled" }, "seq": room.seq, "brief": room.brief, "seats": seats, "posts": posts });
        if let Some(rounds) = &room.rounds {
            page["round"] = json!(rounds.round);
            page["rounds"] = json!(rounds.total);
            page["synthesis"] = json!(rounds.synthesis);
        }
        Ok(page)
    }

    pub fn ack(&mut self, id: &str, who: &Who, through: u64) {
        let Ok(index) = self.index(id) else { return };
        bump(&mut self.rooms[index], who, through);
        if self.rooms[index].published {
            self.write();
        }
    }

    /// Keys of notices still owed, one per recipient. A cursor behind `seq` is not enough: a seat
    /// the posts were not addressed to has no notice, and reconciling one would skip a later wake.
    pub fn unacked_keys(&self) -> Vec<String> {
        self.pending_wakes().into_iter().map(|(_, wake)| wake.key).collect()
    }

    /// Advances a cursor when its notice for `seq` was already taken, so a restart does not send it again.
    pub fn ack_taken(&mut self, keys: &[String]) {
        let mut changed = false;
        for key in keys {
            let Some((id, who, seq)) = parse_wake_key(key) else { continue };
            let Some(room) = self.rooms.iter_mut().find(|room| room.id == id) else { continue };
            let who = if who == CAPTAIN { Who::Captain } else { Who::Seat(who) };
            let before = cursor_of(room, &who);
            bump(room, &who, seq);
            changed |= cursor_of(room, &who) != before;
        }
        if changed {
            self.write();
        }
    }

    /// What an open room still owes. A settled room owes nothing, so a restart does not send its close line again.
    pub fn pending_wakes(&self) -> Vec<(String, Wake)> {
        self.rooms.iter().enumerate().filter(|(_, room)| room.published && room.status == Status::Open).flat_map(|(index, room)| self.wakes(index).into_iter().map(|wake| (room.id.clone(), wake))).collect()
    }

    pub fn rebind(&mut self, id: &str, owner_tab: &str, tab: &str, seats: &[(String, String, String)]) {
        let Some(room) = self.rooms.iter_mut().find(|room| room.id == id) else { return };
        let mut changed = false;
        if room.owner_tab != owner_tab {
            room.owner_tab = owner_tab.to_owned();
            changed = true;
        }
        if room.tab != tab {
            room.tab = tab.to_owned();
            changed = true;
        }
        for (name, task, tab) in seats {
            if let Some(seat) = room.seats.iter_mut().find(|seat| &seat.name == name) {
                if seat.task != *task || seat.tab != *tab {
                    seat.task = task.clone();
                    seat.tab = tab.clone();
                    changed = true;
                }
            }
        }
        if changed && room.published {
            self.write();
        }
    }

    pub fn seat_tasks(&self, id: &str) -> Vec<String> {
        self.get(id).map(|room| room.seats.iter().filter(|seat| !seat.task.is_empty()).map(|seat| seat.task.clone()).collect()).unwrap_or_default()
    }

    fn index(&self, id: &str) -> Result<usize, String> {
        self.rooms.iter().position(|room| room.id == id).ok_or_else(|| format!("no room {id}"))
    }

    fn room_mut(&mut self, id: &str) -> Result<&mut Room, String> {
        let index = self.index(id)?;
        Ok(&mut self.rooms[index])
    }

    /// Who a line wakes. In a debate it skips the seats that owe no post now, so they sleep until the next round.
    fn audience(&self, index: usize, speaker: Option<&str>, to: &[String], captain: bool) -> Result<Vec<String>, String> {
        let room = &self.rooms[index];
        if to.is_empty() {
            let mut names = open_seats(room, speaker);
            names.retain(|name| owes_post(room, name));
            if captain {
                names.push(CAPTAIN.to_owned());
            }
            return Ok(names);
        }
        let mut names = vec![];
        for raw in to {
            let name = raw.trim();
            if name.is_empty() {
                continue;
            }
            if name.eq_ignore_ascii_case(CAPTAIN) {
                if !names.iter().any(|have| have == CAPTAIN) {
                    names.push(CAPTAIN.to_owned());
                }
                continue;
            }
            if !room.seats.iter().any(|seat| seat.name == name) {
                return Err(format!("no seat {name} in {}", room.id));
            }
            if speaker == Some(name) || room.seats.iter().any(|seat| seat.name == name && seat.done) || !owes_post(room, name) {
                continue;
            }
            if !names.iter().any(|have| have == name) {
                names.push(name.to_owned());
            }
        }
        Ok(names)
    }

    fn push(&mut self, index: usize, author: Author, text: String, to: Vec<String>, now: u64) {
        let room = &mut self.rooms[index];
        room.seq += 1;
        room.posts.push(Post { seq: room.seq, author, to, text, at: now, post_id: None });
    }

    fn close_if_full(&mut self, index: usize, now: u64) -> bool {
        if self.rooms[index].status != Status::Open {
            return true;
        }
        let cap = self.rooms[index].post_cap;
        if counted(&self.rooms[index]) < cap {
            return false;
        }
        let reason = format!("it reached {cap} posts");
        self.rooms[index].status = Status::Settled;
        let text = close_text(&self.rooms[index], &reason);
        self.push(index, Author::System, text, vec![CAPTAIN.to_owned()], now);
        true
    }

    fn settle_if_all_done(&mut self, index: usize, now: u64) -> bool {
        if self.rooms[index].status != Status::Open || self.rooms[index].seats.iter().any(|seat| !seat.done) {
            return false;
        }
        self.rooms[index].status = Status::Settled;
        let text = close_text(&self.rooms[index], "");
        self.push(index, Author::System, text, vec![CAPTAIN.to_owned()], now);
        true
    }

    /// A seat's post in a debate. The synthesis closes the room; any other post counts toward the
    /// round. True when that closed the room.
    fn seat_spoke(&mut self, index: usize, name: &str, now: u64) -> bool {
        let Some(rounds) = self.rooms[index].rounds.as_mut() else { return false };
        if rounds.synthesizing {
            let text = self.rooms[index].posts.last().map(|post| post.text.clone()).unwrap_or_default();
            return self.end_debate(index, &format!("{name} wrote the synthesis:\n{text}"), now);
        }
        rounds.posted.push(name.to_owned());
        self.end_round_if_due(index, now)
    }

    /// Ends the round once every seat still working has posted in it. A "Round k of N" line then
    /// wakes them all. After the last round the synthesis seat is asked for the synthesis, or the
    /// room closes. A synthesis seat that stops before writing it closes the room too. True when it closed.
    fn end_round_if_due(&mut self, index: usize, now: u64) -> bool {
        let room = &self.rooms[index];
        let Some(rounds) = room.rounds.clone().filter(|_| room.status == Status::Open) else { return false };
        let working = |name: &str| room.seats.iter().any(|seat| seat.name == name && !seat.done);
        if rounds.synthesizing {
            let seat = rounds.synthesis.unwrap_or_default();
            return !working(&seat) && self.end_debate(index, &format!("no synthesis: {seat} stopped before writing it"), now);
        }
        if room.seats.iter().any(|seat| !seat.done && !rounds.posted.contains(&seat.name)) {
            return false;
        }
        let over = match rounds.total {
            1 => "the round is over".to_owned(),
            total => format!("the {total} rounds are over"),
        };
        if rounds.round < rounds.total {
            let to = open_seats(room, None);
            let round = rounds.round + 1;
            self.rooms[index].rounds = Some(Rounds { round, posted: vec![], ..rounds });
            self.push(index, Author::System, format!("Round {round} of {}. Post once with room_post, then end your turn.", rounds.total), to, now);
            return false;
        }
        match rounds.synthesis {
            Some(seat) if working(&seat) => {
                self.rooms[index].rounds = Some(Rounds { synthesizing: true, posted: vec![], synthesis: Some(seat.clone()), ..rounds });
                self.push(index, Author::System, format!("{seat}, {over}. Write the synthesis now: your next post is the synthesis, and it closes the room."), vec![seat], now);
                false
            }
            Some(seat) => self.end_debate(index, &format!("no synthesis: {seat} stopped before writing it"), now),
            None => self.end_debate(index, &over, now),
        }
    }

    /// Closes a room whose debate is over. The close line wakes the Captain.
    fn end_debate(&mut self, index: usize, reason: &str, now: u64) -> bool {
        self.rooms[index].status = Status::Settled;
        let text = close_text(&self.rooms[index], reason);
        self.push(index, Author::System, text, vec![CAPTAIN.to_owned()], now);
        true
    }

    fn wakes(&self, index: usize) -> Vec<Wake> {
        let room = &self.rooms[index];
        let mut wakes = vec![];
        // A seat with no tab yet still gets its wake, with an empty tab, and it stays owed until one binds.
        for seat in room.seats.iter().filter(|seat| !seat.done) {
            if let Some(wake) = wake_for(room, Who::Seat(seat.name.clone()), seat.cursor, &seat.tab) {
                wakes.push(wake);
            }
        }
        if !room.owner_tab.is_empty() {
            if let Some(wake) = wake_for(room, Who::Captain, room.owner_cursor, &room.owner_tab) {
                wakes.push(wake);
            }
        }
        wakes
    }

    /// The author's own post does not wake them. When nothing earlier is still waiting for them,
    /// their cursor moves past it so a later notice does not repeat it.
    fn quiet_author(&mut self, index: usize, author: &Author) {
        let seq = self.rooms[index].seq;
        let who = match author {
            Author::Seat { name } => Who::Seat(name.clone()),
            Author::Card => Who::Captain,
            Author::User | Author::System => return,
        };
        if self.wakes(index).iter().any(|wake| wake.who == who) {
            return;
        }
        bump(&mut self.rooms[index], &who, seq);
    }

    fn write(&self) {
        let Some(path) = &self.path else { return };
        let saved = Saved { next: self.next, rooms: self.rooms.iter().filter(|room| room.published).cloned().collect() };
        let wrote = (|| {
            use std::io::Write;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            let _ = std::fs::remove_file(&tmp);
            let written = crate::private::file().create_new(true).open(&tmp).and_then(|mut file| {
                file.write_all(&serde_json::to_vec_pretty(&saved).expect("rooms serialize"))?;
                file.sync_all()
            });
            if let Err(e) = written {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            std::fs::rename(&tmp, path)?;
            Ok(())
        })();
        if let Err(e) = wrote {
            eprintln!("pinned: rooms did not save: {e}");
        }
    }
}

fn keep_aside(path: &Path, error: &str) {
    let aside = path.with_extension("json.bad");
    eprintln!("pinned: {FILE} is unreadable ({error}); kept aside as {FILE}.bad");
    if let Err(e) = std::fs::rename(path, aside) {
        eprintln!("pinned: {FILE} could not be kept aside ({e}); rooms will not save over it");
    }
}

pub fn id_in(env: &[(String, String)]) -> Option<String> {
    env.iter().find(|(key, _)| key == ENV).map(|(_, value)| value.clone()).filter(|value| !value.is_empty())
}

pub fn request_id(room: &str, name: &str) -> String {
    format!("room:{room}:{name}")
}

/// `(room id, seat name)` from a task's `clientRequestId`, or nothing when it is not a seat.
pub fn seat_of(client_request_id: &str) -> Option<(&str, &str)> {
    let rest = client_request_id.strip_prefix("room:")?;
    let (id, name) = rest.split_once(':')?;
    (!id.is_empty() && !name.is_empty() && !name.contains(':')).then_some((id, name))
}

pub fn wake_key(room: &str, who: &str, seq: u64) -> String {
    format!("room:{room}:{who}:{seq}")
}

pub fn parse_wake_key(key: &str) -> Option<(String, String, u64)> {
    let rest = key.strip_prefix("room:")?;
    let (id, rest) = rest.split_once(':')?;
    let (who, seq) = rest.rsplit_once(':')?;
    let seq = seq.parse().ok()?;
    (!id.is_empty() && !who.is_empty()).then_some((id.to_owned(), who.to_owned(), seq))
}

pub fn check_brief(raw: &str) -> Result<String, String> {
    let brief = raw.trim();
    if brief.is_empty() {
        return Err("a room needs a brief".into());
    }
    if brief.chars().count() > TEXT_MAX {
        return Err(format!("a brief is at most {TEXT_MAX} characters"));
    }
    Ok(brief.to_owned())
}

pub fn check_names(names: &[String]) -> Result<Vec<String>, String> {
    if names.len() < MIN_SEATS {
        return Err("a room needs at least 2 seats; one worker is a delegate_task".into());
    }
    if names.len() > MAX_SEATS {
        return Err(format!("a room seats at most {MAX_SEATS} workers"));
    }
    let mut out = vec![];
    for name in names {
        let name = check_name(name)?;
        if out.iter().any(|have: &String| have == &name) {
            return Err(format!("seat {name} is listed twice"));
        }
        out.push(name);
    }
    Ok(out)
}

/// The debate a room opens with. Its seat posts and the synthesis must fit under the post cap, so
/// the cap never cuts a debate off halfway.
fn check_rounds(rounds: Option<u32>, synthesis: Option<&str>, names: &[String], post_cap: u32) -> Result<Option<Rounds>, String> {
    let Some(total) = rounds else {
        return match synthesis {
            Some(_) => Err("a synthesis needs rounds".into()),
            None => Ok(None),
        };
    };
    if !(1..=MAX_ROUNDS).contains(&total) {
        return Err(format!("rounds is 1 to {MAX_ROUNDS}, not {total}"));
    }
    let synthesis = synthesis.map(str::trim);
    if let Some(seat) = synthesis.filter(|seat| !names.iter().any(|name| name == seat)) {
        return Err(format!("synthesis \"{seat}\" is not one of the seats"));
    }
    let needed = total * names.len() as u32 + u32::from(synthesis.is_some());
    if needed > post_cap {
        let plus = if synthesis.is_some() { " and the synthesis" } else { "" };
        return Err(format!("{} of {} seats{plus} need {needed} posts, over the post cap of {post_cap}. Raise postCap or run fewer rounds.", rounds_of(total), names.len()));
    }
    Ok(Some(Rounds { total, round: 1, posted: vec![], synthesis: synthesis.map(str::to_owned), synthesizing: false }))
}

/// A seat posts once a round. Once the rounds are over, only the synthesis seat posts.
fn check_turn(rounds: &Rounds, name: &str) -> Result<(), String> {
    if rounds.synthesizing {
        let seat = rounds.synthesis.as_deref().unwrap_or_default();
        return match seat == name {
            true => Ok(()),
            false => Err(format!("the rounds are over: only {seat} posts now, with the synthesis")),
        };
    }
    if rounds.posted.iter().any(|posted| posted == name) {
        return Err(format!("{name} already posted in round {} of {}. End your turn and wait: the next round arrives on its own.", rounds.round, rounds.total));
    }
    Ok(())
}

/// A seat's task. With `rounds`, it also says the seat posts once a round and who writes the synthesis.
pub fn seat_brief(id: &str, name: &str, others: &[String], brief: &str, rounds: Option<u32>, synthesis: Option<&str>) -> String {
    let with = match others {
        [] => "no one else".to_owned(),
        [one] => one.clone(),
        names => format!("{} and {}", names[..names.len() - 1].join(", "), names[names.len() - 1]),
    };
    let mut text = format!("You sit in room {id} as {name}, with {with}.\n\n{brief}\n\nSpeak with room_post. A reply that stays only in this session is invisible to the room. End your turn after you post. The next messages arrive on their own. Do not poll. When your part is done, call room_done with a short summary.");
    if let Some(total) = rounds {
        text.push_str(&format!("\n\nThe room runs in {}. Post once per round with room_post, then end your turn: the next round arrives on its own.", rounds_of(total)));
        text.push_str(&match synthesis {
            Some(seat) if seat == name => " After the last round you write the synthesis, and that post closes the room.".to_owned(),
            Some(seat) => format!(" After the last round, {seat} writes the synthesis."),
            None => " The room closes when the last round ends.".to_owned(),
        });
    }
    text
}

fn check_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    let len = name.chars().count();
    if len == 0 || len > NAME_MAX {
        return Err(format!("a seat name is 1 to {NAME_MAX} characters"));
    }
    if name.chars().any(|c| c == ':' || c == '\n' || c == '\r') {
        return Err(format!("seat name \"{name}\" cannot contain a colon or a new line"));
    }
    if name.eq_ignore_ascii_case(CAPTAIN) || name.eq_ignore_ascii_case("you") {
        return Err(format!("seat name \"{name}\" is reserved"));
    }
    Ok(name.to_owned())
}

fn check_text(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("a post needs text".into());
    }
    if text.chars().count() > TEXT_MAX {
        return Err(format!("a post is at most {TEXT_MAX} characters"));
    }
    Ok(text.to_owned())
}

fn rounds_of(total: u32) -> String {
    match total {
        1 => "1 round".to_owned(),
        total => format!("{total} rounds"),
    }
}

fn one_line(text: &str) -> String {
    let text = text.trim().replace(['\n', '\r'], " ");
    match text.chars().count() > 400 {
        true => text.chars().take(400).collect(),
        false => text,
    }
}

fn open_seats(room: &Room, except: Option<&str>) -> Vec<String> {
    room.seats.iter().filter(|seat| !seat.done && Some(seat.name.as_str()) != except).map(|seat| seat.name.clone()).collect()
}

/// In a debate, a seat that has posted in the round owes nothing until the next one. Once the
/// rounds are over, only the synthesis seat owes a post.
fn owes_post(room: &Room, name: &str) -> bool {
    match &room.rounds {
        None => true,
        Some(rounds) if rounds.synthesizing => rounds.synthesis.as_deref() == Some(name),
        Some(rounds) => !rounds.posted.iter().any(|posted| posted == name),
    }
}

fn counted(room: &Room) -> u32 {
    room.posts.iter().filter(|post| !matches!(post.author, Author::System)).count() as u32
}

fn close_text(room: &Room, reason: &str) -> String {
    let mut lines = vec![format!("{} is closed", room.id)];
    if !reason.is_empty() {
        lines.push(reason.to_owned());
    }
    for seat in &room.seats {
        if let Some(summary) = &seat.summary {
            lines.push(format!("{}: {summary}", seat.name));
        }
    }
    lines.join("\n")
}

fn label(author: &Author) -> String {
    match author {
        Author::Seat { name } => name.clone(),
        Author::Card => "Captain".into(),
        Author::User => "You".into(),
        Author::System => "room".into(),
    }
}

pub fn line_of(post: &Post) -> String {
    match &post.author {
        Author::System => post.text.clone(),
        author => format!("{}: {}", label(author), post.text),
    }
}

fn wake_for(room: &Room, who: Who, cursor: u64, tab: &str) -> Option<Wake> {
    let pending: Vec<&Post> = room.posts.iter().filter(|post| post.seq > cursor).collect();
    if !pending.iter().any(|post| post.to.iter().any(|to| to == who.key())) {
        return None;
    }
    let through = pending.last()?.seq;
    let mut lines = vec![format!("[room {}]", room.id)];
    lines.extend(pending.iter().map(|post| line_of(post)));
    Some(Wake { key: wake_key(&room.id, who.key(), through), tab: tab.to_owned(), who, through, text: lines.join("\n") })
}

fn bump(room: &mut Room, who: &Who, through: u64) {
    match who {
        Who::Seat(name) => {
            if let Some(seat) = room.seats.iter_mut().find(|seat| &seat.name == name) {
                seat.cursor = seat.cursor.max(through);
            }
        }
        Who::Captain => room.owner_cursor = room.owner_cursor.max(through),
    }
}

fn cursor_of(room: &Room, who: &Who) -> u64 {
    match who {
        Who::Seat(name) => room.seats.iter().find(|seat| &seat.name == name).map(|seat| seat.cursor).unwrap_or(0),
        Who::Captain => room.owner_cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    fn open(store: &mut Store) -> String {
        store.open("/app", "tab-captain", "Fix the parser.", &names(&["Ana", "Beto"]), None, None, None, 1).unwrap()
    }

    fn bind(store: &mut Store, id: &str) {
        store.bind_seat(id, "Ana", "task-ana", "tab-ana").unwrap();
        store.bind_seat(id, "Beto", "task-beto", "tab-beto").unwrap();
        store.set_tab(id, "tab-room");
        store.publish(id).unwrap();
    }

    /// A published room that debates in `rounds`, each seat bound to its own task and tab.
    fn debate(store: &mut Store, seats: &[&str], rounds: u32, synthesis: Option<&str>, post_cap: Option<u32>) -> String {
        let id = store.open("/app", "tab-captain", "Which parser?", &names(seats), post_cap, Some(rounds), synthesis, 1).unwrap();
        for seat in seats {
            store.bind_seat(&id, seat, &format!("task-{seat}"), &format!("tab-{seat}")).unwrap();
        }
        store.set_tab(&id, "tab-room");
        store.publish(&id).unwrap();
        id
    }

    /// Acks every wake, as `deliver_room` does once it queues them.
    fn acked(store: &mut Store, id: &str, outcome: Outcome) -> Outcome {
        for wake in &outcome.wakes {
            store.ack(id, &wake.who, wake.through);
        }
        outcome
    }

    fn say(store: &mut Store, id: &str, name: &str, text: &str) -> Result<Outcome, String> {
        let outcome = store.post(id, Speech::Seat { name: name.into(), text: text.into(), to: vec![] }, 2)?;
        Ok(acked(store, id, outcome))
    }

    fn woken(outcome: &Outcome) -> Vec<&str> {
        outcome.wakes.iter().map(|wake| wake.who.key()).collect()
    }

    fn last_line(store: &Store, id: &str) -> String {
        store.get(id).unwrap().posts.last().unwrap().text.clone()
    }

    #[test]
    fn a_debate_that_cannot_run_does_not_open() {
        let mut store = Store::load(None);
        let two = names(&["Ana", "Beto"]);
        let four = names(&["Ana", "Beto", "Cleo", "Dani"]);
        assert_eq!(store.open("/app", "tab-captain", "Debate it.", &two, None, Some(0), None, 1).unwrap_err(), "rounds is 1 to 10, not 0");
        assert_eq!(store.open("/app", "tab-captain", "Debate it.", &two, None, Some(11), None, 1).unwrap_err(), "rounds is 1 to 10, not 11");
        assert_eq!(store.open("/app", "tab-captain", "Debate it.", &two, None, None, Some("Ana"), 1).unwrap_err(), "a synthesis needs rounds");
        assert_eq!(store.open("/app", "tab-captain", "Debate it.", &two, None, Some(2), Some("Zed"), 1).unwrap_err(), "synthesis \"Zed\" is not one of the seats");
        let full = store.open("/app", "tab-captain", "Debate it.", &four, Some(12), Some(3), Some("Ana"), 1).unwrap_err();
        assert!(full.contains("need 13 posts") && full.contains("post cap of 12") && full.contains("postCap"), "{full}");
        assert_eq!(store.open("/app", "tab-captain", "Debate it.", &four, Some(13), Some(3), Some("Ana"), 1).unwrap(), "room-1", "a refusal uses up no id");
    }

    #[test]
    fn a_seat_posts_once_a_round_and_sleeps_until_the_next() {
        let mut store = Store::load(None);
        let id = debate(&mut store, &["Ana", "Beto", "Cleo"], 2, None, None);
        assert_eq!(woken(&say(&mut store, &id, "Ana", "split on the colon").unwrap()), vec!["Beto", "Cleo"]);
        let again = say(&mut store, &id, "Ana", "and another thing").unwrap_err();
        assert!(again.contains("Ana already posted in round 1 of 2") && again.contains("next round arrives on its own"), "{again}");
        assert_eq!(store.read(&id, 0).unwrap()["posts"].as_array().unwrap().len(), 1, "the refused post is not recorded");
        assert_eq!(woken(&say(&mut store, &id, "Beto", "split on the comma").unwrap()), vec!["Cleo"], "Ana has posted, so she sleeps");
        let outcome = say(&mut store, &id, "Cleo", "both work").unwrap();
        assert!(last_line(&store, &id).starts_with("Round 2 of 2"), "{}", last_line(&store, &id));
        assert_eq!(woken(&outcome), vec!["Ana", "Beto", "Cleo"], "the round line wakes each seat once, and not the Captain");
        let ana = &outcome.wakes[0].text;
        assert!(ana.contains("Beto: split on the comma") && ana.contains("Cleo: both work") && ana.contains("Round 2 of 2"), "{ana}");
        let read = store.read(&id, 0).unwrap();
        assert_eq!((&read["round"], &read["rounds"], &read["synthesis"]), (&json!(2), &json!(2), &Value::Null));
    }

    #[test]
    fn captain_and_user_lines_neither_count_toward_a_round_nor_wake_a_seat_that_posted() {
        let mut store = Store::load(None);
        let id = debate(&mut store, &["Ana", "Beto", "Cleo"], 2, None, None);
        say(&mut store, &id, "Ana", "split on the colon").unwrap();
        let card = store.post(&id, Speech::Card { text: "keep it short".into(), to: vec![] }, 3).unwrap();
        assert_eq!(woken(&acked(&mut store, &id, card)), vec!["Beto", "Cleo"]);
        let user = store.post(&id, Speech::User { text: "and simple".into() }, 4).unwrap();
        assert_eq!(woken(&acked(&mut store, &id, user)), vec!["Beto", "Cleo", "captain"]);
        let named = store.post(&id, Speech::Card { text: "Ana, one more?".into(), to: names(&["Ana"]) }, 5).unwrap();
        assert_eq!(woken(&named), Vec::<&str>::new(), "even by name, Ana waits for the next round");
        assert_eq!(store.read(&id, 0).unwrap()["round"], 1);
        say(&mut store, &id, "Beto", "split on the comma").unwrap();
        assert_eq!(store.read(&id, 0).unwrap()["round"], 1, "Cleo has not posted yet");
        say(&mut store, &id, "Cleo", "both work").unwrap();
        assert!(last_line(&store, &id).starts_with("Round 2 of 2"));
        assert_eq!(store.read(&id, 0).unwrap()["round"], 2);
    }

    #[test]
    fn a_seat_that_is_done_no_longer_holds_the_round() {
        for gone in [false, true] {
            let mut store = Store::load(None);
            let id = debate(&mut store, &["Ana", "Beto", "Cleo"], 2, None, None);
            say(&mut store, &id, "Ana", "split on the colon").unwrap();
            say(&mut store, &id, "Beto", "split on the comma").unwrap();
            let outcome = match gone {
                true => store.seat_gone(&id, "Cleo", 3).unwrap(),
                false => store.finish(&id, "Cleo", "nothing to add", 3).unwrap(),
            };
            assert!(!outcome.closed);
            assert!(last_line(&store, &id).starts_with("Round 2 of 2"), "{}", last_line(&store, &id));
            let seats: Vec<&str> = woken(&outcome).into_iter().filter(|who| *who != CAPTAIN).collect();
            assert_eq!(seats, vec!["Ana", "Beto"]);
        }
    }

    #[test]
    fn after_the_last_round_only_the_synthesis_seat_posts_and_its_post_closes_the_room() {
        let mut store = Store::load(None);
        let id = debate(&mut store, &["Ana", "Beto"], 1, Some("Beto"), None);
        say(&mut store, &id, "Ana", "split on the colon").unwrap();
        let outcome = say(&mut store, &id, "Beto", "split on the comma").unwrap();
        let ask = store.get(&id).unwrap().posts.last().unwrap().clone();
        assert!(ask.text.contains("Beto") && ask.text.contains("synthesis"), "{}", ask.text);
        assert_eq!(ask.to, names(&["Beto"]));
        assert_eq!(woken(&outcome), vec!["Beto"]);
        let late = say(&mut store, &id, "Ana", "one more").unwrap_err();
        assert!(late.contains("rounds are over") && late.contains("Beto"), "{late}");
        let outcome = store.post(&id, Speech::Seat { name: "Beto".into(), text: "the synthesis text".into(), to: vec![] }, 3).unwrap();
        assert!(outcome.closed && outcome.rounds_closed);
        assert_eq!(store.get(&id).unwrap().status, Status::Settled);
        let close = last_line(&store, &id);
        assert!(close.contains("room-1 is closed") && close.contains("Beto wrote the synthesis") && close.contains("the synthesis text"), "{close}");
        let captain = outcome.wakes.iter().find(|wake| wake.who == Who::Captain).unwrap();
        assert!(captain.text.contains(&close), "{}", captain.text);
    }

    #[test]
    fn with_no_synthesis_seat_the_last_round_closes_the_room() {
        let mut store = Store::load(None);
        let id = debate(&mut store, &["Ana", "Beto"], 2, None, None);
        say(&mut store, &id, "Ana", "split on the colon").unwrap();
        say(&mut store, &id, "Beto", "split on the comma").unwrap();
        say(&mut store, &id, "Ana", "the colon").unwrap();
        let outcome = say(&mut store, &id, "Beto", "fine, the colon").unwrap();
        assert!(outcome.closed && outcome.rounds_closed);
        assert!(last_line(&store, &id).contains("the 2 rounds are over"), "{}", last_line(&store, &id));
        assert!(outcome.wakes.iter().any(|wake| wake.who == Who::Captain));
    }

    #[test]
    fn a_synthesis_seat_that_stops_closes_the_room_with_no_synthesis() {
        let mut store = Store::load(None);
        let id = debate(&mut store, &["Ana", "Beto"], 1, Some("Beto"), None);
        store.finish(&id, "Beto", "I am out", 2).unwrap();
        let outcome = say(&mut store, &id, "Ana", "split on the colon").unwrap();
        assert!(outcome.closed && outcome.rounds_closed);
        let close = last_line(&store, &id);
        assert!(close.contains("no synthesis") && close.contains("Beto"), "{close}");

        let mut store = Store::load(None);
        let id = debate(&mut store, &["Ana", "Beto"], 1, Some("Beto"), None);
        say(&mut store, &id, "Ana", "split on the colon").unwrap();
        say(&mut store, &id, "Beto", "split on the comma").unwrap();
        let outcome = store.fail_seat(&id, "Beto", "the turn failed", 3).unwrap();
        assert!(outcome.closed && outcome.rounds_closed);
        let close = last_line(&store, &id);
        assert!(close.contains("no synthesis") && close.contains("Beto"), "{close}");
    }

    #[test]
    fn a_synthesis_that_reaches_the_post_cap_closes_as_the_synthesis() {
        let mut store = Store::load(None);
        let id = debate(&mut store, &["Ana", "Beto"], 2, Some("Ana"), Some(5));
        for (name, text) in [("Ana", "colon"), ("Beto", "comma"), ("Ana", "colon"), ("Beto", "colon then")] {
            say(&mut store, &id, name, text).unwrap();
        }
        let outcome = say(&mut store, &id, "Ana", "we split on the colon").unwrap();
        assert!(outcome.closed);
        assert_eq!(counted(store.get(&id).unwrap()), 5);
        let close = last_line(&store, &id);
        assert!(close.contains("Ana wrote the synthesis") && !close.contains("reached"), "{close}");
    }

    #[test]
    fn a_debate_carries_on_from_the_same_round_after_a_restart() {
        let dir = std::env::temp_dir().join(format!("pinned-room-rounds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::load(Some(&dir));
        let id = debate(&mut store, &["Ana", "Beto"], 2, Some("Beto"), None);
        say(&mut store, &id, "Ana", "split on the colon").unwrap();
        let mut reloaded = Store::load(Some(&dir));
        assert!(say(&mut reloaded, &id, "Ana", "again").unwrap_err().contains("already posted in round 1 of 2"));
        say(&mut reloaded, &id, "Beto", "split on the comma").unwrap();
        assert!(last_line(&reloaded, &id).starts_with("Round 2 of 2"));
        let read = reloaded.read(&id, 0).unwrap();
        assert_eq!((&read["round"], &read["synthesis"]), (&json!(2), &json!("Beto")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_room_without_rounds_writes_no_new_keys() {
        let mut store = Store::load(None);
        let id = open(&mut store);
        bind(&mut store, &id);
        store.post(&id, Speech::Seat { name: "Ana".into(), text: "hi".into(), to: vec![] }, 2).unwrap();
        let saved = serde_json::to_value(store.get(&id).unwrap()).unwrap();
        let room_keys = ["id", "project", "owner_tab", "tab", "brief", "status", "seq", "post_cap", "seats", "posts", "owner_cursor", "opened"];
        assert!(saved.as_object().unwrap().keys().all(|key| room_keys.contains(&key.as_str())), "{saved}");
        for seat in saved["seats"].as_array().unwrap() {
            assert!(seat.as_object().unwrap().keys().all(|key| ["name", "task", "tab", "cursor", "done", "summary"].contains(&key.as_str())), "{seat}");
        }
    }

    #[test]
    fn a_seat_brief_tells_the_rounds_and_who_writes_the_synthesis() {
        let plain = seat_brief("room-1", "Ana", &names(&["Beto"]), "Fix it.", None, None);
        assert_eq!(plain, "You sit in room room-1 as Ana, with Beto.\n\nFix it.\n\nSpeak with room_post. A reply that stays only in this session is invisible to the room. End your turn after you post. The next messages arrive on their own. Do not poll. When your part is done, call room_done with a short summary.");
        let ana = seat_brief("room-1", "Ana", &names(&["Beto", "Cleo"]), "Fix it.", Some(3), Some("Beto"));
        assert!(ana.contains("3 rounds") && ana.contains("once per round") && ana.contains("Beto writes the synthesis"), "{ana}");
        let beto = seat_brief("room-1", "Beto", &names(&["Ana", "Cleo"]), "Fix it.", Some(3), Some("Beto"));
        assert!(beto.contains("you write the synthesis") && beto.contains("closes the room"), "{beto}");
    }

    #[test]
    fn seat_names_follow_the_room_rules() {
        assert_eq!(check_names(&names(&["Ana"])).unwrap_err(), "a room needs at least 2 seats; one worker is a delegate_task");
        assert!(check_names(&vec!["A".into(); 13]).unwrap_err().contains("at most 12"));
        assert_eq!(check_names(&names(&["Ana", " Ana "])).unwrap_err(), "seat Ana is listed twice");
        assert_eq!(check_names(&names(&["Captain", "Beto"])).unwrap_err(), "seat name \"Captain\" is reserved");
        assert_eq!(check_names(&names(&["you", "Beto"])).unwrap_err(), "seat name \"you\" is reserved");
        assert!(check_names(&names(&["A:na", "Beto"])).unwrap_err().contains("colon"));
        assert!(check_names(&names(&["A\nna", "Beto"])).unwrap_err().contains("new line"));
        assert_eq!(check_names(&names(&[" Ana ", "Beto"])).unwrap(), names(&["Ana", "Beto"]));
        assert_eq!(check_brief("  ").unwrap_err(), "a room needs a brief");
    }

    #[test]
    fn a_draft_room_consumes_its_id_and_publish_writes_it() {
        let dir = std::env::temp_dir().join(format!("pinned-room-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = Store::load(Some(&dir));
        let id = open(&mut store);
        assert_eq!(id, "room-1");
        let loaded = Store::load(Some(&dir));
        assert!(loaded.get(&id).is_none(), "a room is not on disk until it opens");
        assert_eq!(loaded.next, 1, "the id is consumed even so");
        bind(&mut store, &id);
        let loaded = Store::load(Some(&dir));
        let room = loaded.get(&id).unwrap();
        assert_eq!(room.brief, "Fix the parser.");
        assert_eq!(room.seats[0].task, "task-ana");
        assert!(room.published);
        store.discard(&id);
        assert!(Store::load(Some(&dir)).get(&id).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_post_wakes_the_other_seat_and_not_the_author_or_the_captain() {
        let mut store = Store::load(None);
        let id = open(&mut store);
        bind(&mut store, &id);
        let outcome = store.post(&id, Speech::Seat { name: "Ana".into(), text: "the split is on the colon".into(), to: vec![] }, 2).unwrap();
        assert_eq!(outcome.wakes.len(), 1);
        let wake = &outcome.wakes[0];
        assert_eq!(wake.who, Who::Seat("Beto".into()));
        assert_eq!(wake.text, "[room room-1]\nAna: the split is on the colon");
        assert_eq!(wake.key, "room:room-1:Beto:1");
        assert_eq!(store.get(&id).unwrap().seats[0].cursor, 1, "Ana's own post does not wait for her");
        store.ack(&id, &Who::Seat("Beto".into()), wake.through);
        let outcome = store.post(&id, Speech::Seat { name: "Beto".into(), text: "I will take the tests".into(), to: vec![] }, 3).unwrap();
        assert_eq!(outcome.wakes[0].who, Who::Seat("Ana".into()));
        assert_eq!(outcome.wakes[0].text, "[room room-1]\nBeto: I will take the tests");
        assert!(outcome.wakes.iter().all(|wake| wake.who != Who::Captain));
    }

    #[test]
    fn addressing_one_seat_leaves_the_other_quiet_until_a_later_post_catches_them_up() {
        let mut store = Store::load(None);
        let id = store.open("/app", "tab-captain", "Fix it.", &names(&["Ana", "Beto", "Cleo"]), None, None, None, 1).unwrap();
        for (name, n) in [("Ana", "1"), ("Beto", "2"), ("Cleo", "3")] {
            store.bind_seat(&id, name, &format!("task-{n}"), &format!("tab-{n}")).unwrap();
        }
        store.publish(&id).unwrap();
        let outcome = store.post(&id, Speech::Seat { name: "Ana".into(), text: "only Beto".into(), to: names(&["Beto"]) }, 2).unwrap();
        assert_eq!(outcome.wakes.iter().map(|wake| wake.who.clone()).collect::<Vec<_>>(), vec![Who::Seat("Beto".into())]);
        store.ack(&id, &Who::Seat("Beto".into()), outcome.wakes[0].through);
        let outcome = store.post(&id, Speech::Seat { name: "Ana".into(), text: "everyone now".into(), to: vec![] }, 3).unwrap();
        let cleo = outcome.wakes.iter().find(|wake| wake.who == Who::Seat("Cleo".into())).unwrap();
        assert_eq!(cleo.text, "[room room-1]\nAna: only Beto\nAna: everyone now");
        let beto = outcome.wakes.iter().find(|wake| wake.who == Who::Seat("Beto".into())).unwrap();
        assert_eq!(beto.text, "[room room-1]\nAna: everyone now");
    }

    #[test]
    fn the_captain_hears_a_post_addressed_to_them_and_a_user_and_not_the_chatter() {
        let mut store = Store::load(None);
        let id = open(&mut store);
        bind(&mut store, &id);
        let outcome = store.post(&id, Speech::Seat { name: "Ana".into(), text: "captain, the split is wrong".into(), to: names(&["captain"]) }, 2).unwrap();
        assert_eq!(outcome.wakes.iter().map(|wake| wake.who.clone()).collect::<Vec<_>>(), vec![Who::Captain]);
        store.ack(&id, &Who::Captain, outcome.seq);
        let outcome = store.post(&id, Speech::User { text: "Ship it.".into() }, 3).unwrap();
        let who: Vec<_> = outcome.wakes.iter().map(|wake| wake.who.clone()).collect();
        assert_eq!(who, vec![Who::Seat("Ana".into()), Who::Seat("Beto".into()), Who::Captain]);
        assert!(outcome.wakes.iter().all(|wake| wake.text.contains("You: Ship it.")));
    }

    #[test]
    fn both_seats_finishing_closes_the_room_and_a_later_post_is_refused() {
        let mut store = Store::load(None);
        let id = open(&mut store);
        bind(&mut store, &id);
        store.post(&id, Speech::Seat { name: "Ana".into(), text: "done my part".into(), to: vec![] }, 2).unwrap();
        let beto = store.get(&id).unwrap().seats[1].cursor;
        store.ack(&id, &Who::Seat("Beto".into()), 1);
        assert!(store.finish(&id, "Ana", "parser splits", 3).unwrap().wakes.iter().any(|wake| wake.who == Who::Captain));
        assert_eq!(store.get(&id).unwrap().status, Status::Open);
        let outcome = store.finish(&id, "Beto", "tests cover it", 4).unwrap();
        assert!(outcome.closed);
        let captain = outcome.wakes.iter().find(|wake| wake.who == Who::Captain).unwrap();
        assert!(captain.text.contains("Beto finished: tests cover it"), "{}", captain.text);
        assert!(captain.text.contains("room-1 is closed"), "{}", captain.text);
        assert!(captain.text.contains("Ana: parser splits") && captain.text.contains("Beto: tests cover it"), "{}", captain.text);
        assert_eq!(store.post(&id, Speech::Seat { name: "Ana".into(), text: "more".into(), to: vec![] }, 5).unwrap_err(), "room-1 is closed");
        let read = store.read(&id, 0).unwrap();
        assert_eq!(read["status"], "settled");
        assert!(read["posts"].as_array().unwrap().len() > 1);
        assert_eq!(store.get(&id).unwrap().seats[1].cursor.max(beto), store.get(&id).unwrap().seats[1].cursor);
    }

    #[test]
    fn the_post_cap_counts_speech_and_not_the_close_line() {
        let mut store = Store::load(None);
        let id = store.open("/app", "tab-captain", "Fix it.", &names(&["Ana", "Beto"]), Some(1), None, None, 1).unwrap();
        bind(&mut store, &id);
        let outcome = store.post(&id, Speech::Seat { name: "Ana".into(), text: "only one".into(), to: vec![] }, 2).unwrap();
        assert!(outcome.closed);
        assert_eq!(store.get(&id).unwrap().status, Status::Settled);
        assert_eq!(counted(store.get(&id).unwrap()), 1);
        assert!(store.finish(&id, "Ana", "late", 3).unwrap_err().contains("closed"));
    }

    #[test]
    fn a_done_seat_is_not_woken() {
        let mut store = Store::load(None);
        let id = open(&mut store);
        bind(&mut store, &id);
        store.finish(&id, "Ana", "parser splits", 2).unwrap();
        let outcome = store.post(&id, Speech::Seat { name: "Beto".into(), text: "still here".into(), to: vec![] }, 3).unwrap();
        assert!(outcome.wakes.iter().all(|wake| wake.who != Who::Seat("Ana".into())));
        assert_eq!(store.post(&id, Speech::Seat { name: "Ana".into(), text: "again".into(), to: vec![] }, 4).unwrap_err(), "Ana already finished");
    }

    #[test]
    fn a_seat_request_id_round_trips_and_a_taken_notice_advances_the_cursor() {
        assert_eq!(seat_of(&request_id("room-1", "Ana")), Some(("room-1", "Ana")));
        assert_eq!(seat_of("task-9"), None);
        assert_eq!(parse_wake_key(&wake_key("room-1", "Ana", 4)), Some(("room-1".into(), "Ana".into(), 4)));
        let mut store = Store::load(None);
        let id = open(&mut store);
        bind(&mut store, &id);
        store.post(&id, Speech::Seat { name: "Ana".into(), text: "hi".into(), to: vec![] }, 2).unwrap();
        let key = wake_key(&id, "Beto", 1);
        assert_eq!(store.unacked_keys(), vec![key.clone()]);
        store.ack_taken(&[key]);
        assert!(store.unacked_keys().is_empty());
        assert_eq!(store.pending_wakes(), vec![]);
    }

    #[test]
    fn a_post_to_a_seat_with_no_tab_waits_and_arrives_when_the_seat_is_bound() {
        let mut store = Store::load(None);
        let id = open(&mut store);
        store.bind_seat(&id, "Ana", "task-ana", "tab-ana").unwrap();
        store.publish(&id).unwrap();
        let outcome = store.post(&id, Speech::Seat { name: "Ana".into(), text: "Beto, take the tests".into(), to: names(&["Beto"]) }, 2).unwrap();
        let beto = outcome.wakes.iter().find(|wake| wake.who == Who::Seat("Beto".into())).unwrap();
        assert_eq!(beto.tab, "", "with no tab the wake waits instead of going out");
        assert!(store.unacked_keys().contains(&wake_key(&id, "Beto", 1)), "the wake stays owed");
        store.bind_seat(&id, "Beto", "task-beto", "tab-beto").unwrap();
        let (_, wake) = store.pending_wakes().into_iter().find(|(_, wake)| wake.who == Who::Seat("Beto".into())).unwrap();
        assert_eq!(wake.tab, "tab-beto");
        assert_eq!(wake.text, "[room room-1]\nAna: Beto, take the tests");
    }

    #[test]
    fn a_saved_room_loads_back_with_its_state_seats_and_pending_wakes() {
        let dir = std::env::temp_dir().join(format!("pinned-room-reload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = Store::load(Some(&dir));
        let id = open(&mut store);
        bind(&mut store, &id);
        store.post(&id, Speech::Seat { name: "Ana".into(), text: "Beto, look".into(), to: names(&["Beto"]) }, 2).unwrap();
        let reloaded = Store::load(Some(&dir));
        let room = reloaded.get(&id).unwrap();
        assert_eq!(room.status, Status::Open);
        assert_eq!(room.owner_tab, "tab-captain");
        assert_eq!(room.tab, "tab-room");
        assert_eq!(room.seats.iter().map(|seat| seat.name.as_str()).collect::<Vec<_>>(), vec!["Ana", "Beto"]);
        assert_eq!(room.seats[1].tab, "tab-beto");
        assert_eq!(room.seats[0].cursor, 1);
        assert_eq!(room.posts.len(), 1);
        let pending = reloaded.pending_wakes();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.who, Who::Seat("Beto".into()));
        assert_eq!(pending[0].1.tab, "tab-beto");
        assert_eq!(pending[0].1.key, "room:room-1:Beto:1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_settled_room_owes_no_wake() {
        let mut store = Store::load(None);
        let id = open(&mut store);
        bind(&mut store, &id);
        store.close(&id, "enough", 2).unwrap();
        assert!(store.pending_wakes().is_empty(), "the close line is not owed once the room is settled");
    }

    #[test]
    fn a_corrupt_rooms_file_is_kept_aside_and_a_fresh_one_takes_its_place() {
        let dir = std::env::temp_dir().join(format!("pinned-room-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), b"{ not json").unwrap();
        let mut store = Store::load(Some(&dir));
        assert!(store.published_ids().is_empty(), "nothing could be read out of it");
        assert!(dir.join("rooms.json.bad").exists(), "the unreadable file is kept aside");
        assert!(!dir.join(FILE).exists());
        let id = open(&mut store);
        store.publish(&id).unwrap();
        assert!(Store::load(Some(&dir)).get(&id).is_some(), "the next save writes a clean file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rooms_file_from_an_older_version_loads() {
        let dir = std::env::temp_dir().join(format!("pinned-room-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = r#"{
  "next": 3,
  "rooms": [
    {
      "id": "room-2",
      "project": "/app",
      "owner_tab": "tab-captain",
      "brief": "Fix it.",
      "status": "open",
      "seq": 2,
      "seats": [{ "name": "Ana" }, { "name": "Beto" }],
      "opened": 9,
      "posts": [{ "seq": 1, "author": { "who": "seat", "name": "Ana" }, "text": "hi", "at": 9 }]
    }
  ]
}"#;
        std::fs::write(dir.join(FILE), legacy).unwrap();
        let mut store = Store::load(Some(&dir));
        let room = store.get("room-2").unwrap();
        assert_eq!(room.status, Status::Open);
        assert_eq!(room.post_cap, DEFAULT_POST_CAP);
        assert_eq!(room.tab, "");
        assert_eq!(room.owner_cursor, 0);
        assert_eq!(room.posts[0].to, Vec::<String>::new());
        assert_eq!(room.seats[0].task, "");
        assert_eq!(room.seats[0].cursor, 0);
        assert!(!room.seats[0].done);
        assert!(room.published);
        assert_eq!(store.next, 3);
        let read = store.read("room-2", 0).unwrap();
        assert!(["round", "rounds", "synthesis"].iter().all(|key| read.get(key).is_none()), "{read}");
        store.post("room-2", Speech::Seat { name: "Ana".into(), text: "again".into(), to: vec![] }, 10).unwrap();
        store.post("room-2", Speech::Seat { name: "Ana".into(), text: "and again".into(), to: vec![] }, 11).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
