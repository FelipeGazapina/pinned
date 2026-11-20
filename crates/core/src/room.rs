//! One shared thread several workers sit in. The project's Captain opens it with `room_open`.
//! Speech, the brief and each seat's summary live here. A seat's tools, diffs and worktree stay
//! on that seat's own tab. Wakes leave through the notice queue: this module only says who to tell.
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
    /// False until every seat has a task. Absent on disk, so a loaded room is published.
    #[serde(default = "published_yes", skip_serializing)]
    pub published: bool,
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
}

/// What `room_post` says, before the room resolves who it wakes.
#[derive(Clone, Debug)]
pub enum Speech {
    Seat { name: String, text: String, to: Vec<String> },
    Card { text: String, to: Vec<String> },
    User { text: String },
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

    /// Starts a room in memory and consumes its id. It is not on disk until `publish`.
    pub fn open(&mut self, project: &str, owner_tab: &str, brief: &str, names: &[String], post_cap: Option<u32>, now: u64) -> Result<String, String> {
        let brief = check_brief(brief)?;
        let names = check_names(names)?;
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
            post_cap: post_cap.map_or(DEFAULT_POST_CAP, |cap| cap.clamp(1, MAX_POST_CAP)),
            seats: names.into_iter().map(|name| Seat { name, task: String::new(), tab: String::new(), cursor: 0, done: false, summary: None }).collect(),
            posts: vec![],
            owner_cursor: 0,
            opened: now,
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
        let index = self.index(id)?;
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
                (Author::Seat { name: name.clone() }, check_text(&text)?, self.audience(index, Some(&name), &to, false)?)
            }
            Speech::Card { text, to } => (Author::Card, check_text(&text)?, self.audience(index, None, &to, false)?),
            Speech::User { text } => (Author::User, check_text(&text)?, self.audience(index, None, &[], true)?),
        };
        self.push(index, author.clone(), text, to, now);
        let closed = self.close_if_full(index, now);
        let wakes = self.wakes(index);
        self.quiet_author(index, &author);
        if self.rooms[index].published {
            self.write();
        }
        Ok(Outcome { seq: self.rooms[index].seq, closed, wakes })
    }

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
        let closed = self.settle_if_all_done(index, now) || self.close_if_full(index, now);
        let wakes = self.wakes(index);
        if self.rooms[index].published {
            self.write();
        }
        Ok(Outcome { seq: self.rooms[index].seq, closed, wakes })
    }

    /// A seat whose task died. Marks it done so the room can still finish.
    pub fn fail_seat(&mut self, id: &str, name: &str, error: &str, now: u64) -> Result<Outcome, String> {
        let index = self.index(id)?;
        if self.rooms[index].status != Status::Open {
            return Ok(Outcome { wakes: vec![], closed: true, seq: self.rooms[index].seq });
        }
        let Some(seat) = self.rooms[index].seats.iter_mut().find(|seat| seat.name == name) else {
            return Ok(Outcome { wakes: vec![], closed: false, seq: self.rooms[index].seq });
        };
        if seat.done {
            return Ok(Outcome { wakes: vec![], closed: false, seq: self.rooms[index].seq });
        }
        let error = one_line(error);
        seat.done = true;
        seat.summary = Some(error.clone());
        let audience = self.audience(index, Some(name), &[], true)?;
        self.push(index, Author::System, format!("{name} failed: {error}"), audience, now);
        let closed = self.settle_if_all_done(index, now);
        let wakes = self.wakes(index);
        if self.rooms[index].published {
            self.write();
        }
        Ok(Outcome { seq: self.rooms[index].seq, closed, wakes })
    }

    /// The seat's task is already over, so the room must not wait on it. No finish line: the task's
    /// own report already said why. When that was the last open seat, the room closes.
    pub fn seat_gone(&mut self, id: &str, name: &str, now: u64) -> Result<Outcome, String> {
        let index = self.index(id)?;
        if self.rooms[index].status != Status::Open {
            return Ok(Outcome { wakes: vec![], closed: true, seq: self.rooms[index].seq });
        }
        if let Some(seat) = self.rooms[index].seats.iter_mut().find(|seat| seat.name == name) {
            seat.done = true;
        }
        let closed = self.settle_if_all_done(index, now);
        let wakes = self.wakes(index);
        if self.rooms[index].published {
            self.write();
        }
        Ok(Outcome { seq: self.rooms[index].seq, closed, wakes })
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
        Ok(Outcome { seq: self.rooms[index].seq, closed: true, wakes })
    }

    pub fn read(&self, id: &str, since: u64) -> Result<Value, String> {
        let room = self.get(id).ok_or_else(|| format!("no room {id}"))?;
        let posts: Vec<Value> = room
            .posts
            .iter()
            .filter(|post| post.seq > since)
            .map(|post| json!({ "seq": post.seq, "author": label(&post.author), "text": post.text, "at": post.at }))
            .collect();
        let seats: Vec<Value> = room.seats.iter().map(|seat| json!({ "name": seat.name, "done": seat.done, "summary": seat.summary })).collect();
        Ok(json!({ "id": room.id, "status": match room.status { Status::Open => "open", Status::Settled => "settled" }, "seq": room.seq, "brief": room.brief, "seats": seats, "posts": posts }))
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

    pub fn pending_wakes(&self) -> Vec<(String, Wake)> {
        self.rooms.iter().enumerate().filter(|(_, room)| room.published).flat_map(|(index, room)| self.wakes(index).into_iter().map(|wake| (room.id.clone(), wake))).collect()
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

    fn audience(&self, index: usize, speaker: Option<&str>, to: &[String], captain: bool) -> Result<Vec<String>, String> {
        let room = &self.rooms[index];
        if to.is_empty() {
            let mut names = open_seats(room, speaker);
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
            if speaker == Some(name) || room.seats.iter().any(|seat| seat.name == name && seat.done) {
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
        room.posts.push(Post { seq: room.seq, author, to, text, at: now });
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

    fn wakes(&self, index: usize) -> Vec<Wake> {
        let room = &self.rooms[index];
        let mut wakes = vec![];
        for seat in room.seats.iter().filter(|seat| !seat.done && !seat.tab.is_empty()) {
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

pub fn seat_brief(id: &str, name: &str, others: &[String], brief: &str) -> String {
    let with = match others {
        [] => "no one else".to_owned(),
        [one] => one.clone(),
        names => format!("{} and {}", names[..names.len() - 1].join(", "), names[names.len() - 1]),
    };
    format!("You sit in room {id} as {name}, with {with}.\n\n{brief}\n\nSpeak with room_post. A reply that stays only in this session is invisible to the room. End your turn after you post. The next messages arrive on their own. Do not poll. When your part is done, call room_done with a short summary.")
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
        store.open("/app", "tab-captain", "Fix the parser.", &names(&["Ana", "Beto"]), None, 1).unwrap()
    }

    fn bind(store: &mut Store, id: &str) {
        store.bind_seat(id, "Ana", "task-ana", "tab-ana").unwrap();
        store.bind_seat(id, "Beto", "task-beto", "tab-beto").unwrap();
        store.set_tab(id, "tab-room");
        store.publish(id).unwrap();
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
        let id = store.open("/app", "tab-captain", "Fix it.", &names(&["Ana", "Beto", "Cleo"]), None, 1).unwrap();
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
        let id = store.open("/app", "tab-captain", "Fix it.", &names(&["Ana", "Beto"]), Some(1), 1).unwrap();
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
}
