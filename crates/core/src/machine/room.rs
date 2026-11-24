//! A room is one shared thread. The Captain opens it and seats workers who must read each other.
//! Speech lives in `crate::room`. Each seat is an ordinary delegated task on this machine, and a
//! post wakes the others through the notice queue. The room tab itself has no Claude session.

use super::Machine;
use crate::agent::{role_of, Role, PROJECT_ENV};
use crate::history;
use crate::team_tools::{RoomClose, RoomDone, RoomOpen, RoomPost, RoomRead, RoomSeatSpec};
use crate::wire::ThreadPage;
use crate::{room, thread};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Who is calling a room tool, once the room is known.
struct Place {
    id: String,
    /// Set when the caller sits in the room. None is the Captain.
    seat: Option<String>,
}

impl Machine {
    pub(super) fn room_open(&self, tab: Option<&str>, call: RoomOpen) -> Result<String, String> {
        let tab = tab.filter(|id| !id.is_empty()).ok_or("only a project's Captain opens a room")?;
        let project = self.captain_project(tab)?;
        let names = room::check_names(&call.seats.iter().map(|seat| seat.name.clone()).collect::<Vec<_>>())?;
        let brief = room::check_brief(&call.brief)?;
        let isolations = call.seats.iter().map(|seat| isolation_of(seat.isolation.as_deref())).collect::<Result<Vec<_>, _>>()?;
        let limits = self.caller_limits(tab)?;
        let need = names.len();
        if need > limits.parallel as usize {
            return Err(seats_do_not_fit(need, limits.parallel));
        }
        let working = self.delegates.working_in(&project);
        if working + need > limits.parallel as usize {
            return Err(super::workers_full(working));
        }
        let id = self.rooms.lock().unwrap().open(&project.display().to_string(), tab, &brief, &names, call.post_cap, super::unix_millis())?;
        let room_tab = match self.add_room_tab(tab, &id, &brief, &project) {
            Ok(room_tab) => room_tab,
            Err(error) => {
                self.rooms.lock().unwrap().discard(&id);
                return Err(error);
            }
        };
        self.rooms.lock().unwrap().set_tab(&id, &room_tab);
        let mut started: Vec<(String, String, String)> = vec![];
        for ((seat, name), isolation) in call.seats.iter().zip(&names).zip(&isolations) {
            match self.start_seat(tab, &id, &brief, name, &names, seat, isolation) {
                Ok((task, seat_tab)) => {
                    if let Err(error) = self.rooms.lock().unwrap().bind_seat(&id, name, &task, &seat_tab) {
                        started.push((name.clone(), seat_tab, task));
                        self.rollback_room(tab, &id, &room_tab, &started);
                        return Err(error);
                    }
                    started.push((name.clone(), seat_tab, task));
                }
                Err(error) => {
                    self.rollback_room(tab, &id, &room_tab, &started);
                    return Err(error);
                }
            }
        }
        self.rooms.lock().unwrap().publish(&id)?;
        self.emit(super::Event::Thread { tab: room_tab.clone() });
        self.emit_tabs();
        let seats: Vec<Value> = started.iter().map(|(name, tab, task)| json!({ "name": name, "tab": tab, "task": task })).collect();
        serde_json::to_string(&json!({ "id": id, "tabId": room_tab, "seats": seats })).map_err(|e| e.to_string())
    }

    pub(super) fn room_post(&self, tab: Option<&str>, task: Option<&str>, call: RoomPost) -> Result<String, String> {
        let place = self.place(tab, task, call.room.as_deref())?;
        let speech = match &place.seat {
            Some(name) => room::Speech::Seat { name: name.clone(), text: call.text, to: call.to },
            None => room::Speech::Card { text: call.text, to: call.to },
        };
        let outcome = self.rooms.lock().unwrap().post_with(&place.id, speech, call.id.as_deref(), super::unix_millis())?;
        self.after_speech(&place.id, outcome, true)
    }

    pub(super) fn room_read(&self, tab: Option<&str>, task: Option<&str>, call: RoomRead) -> Result<String, String> {
        let place = self.place(tab, task, call.room.as_deref())?;
        let page = self.rooms.lock().unwrap().read(&place.id, call.since.unwrap_or(0))?;
        serde_json::to_string(&page).map_err(|e| e.to_string())
    }

    pub(super) fn room_done(&self, tab: Option<&str>, task: Option<&str>, call: RoomDone) -> Result<String, String> {
        let place = self.place(tab, task, call.room.as_deref())?;
        let Some(name) = place.seat else {
            return Err("only a seat finishes with room_done".into());
        };
        let outcome = self.rooms.lock().unwrap().finish(&place.id, &name, &call.summary, super::unix_millis())?;
        // The seat's task stays up until its turn ends. Cancelling it here would settle the task
        // before that Done, and the seat would keep no workers-at-once slot to release on purpose.
        self.after_speech(&place.id, outcome, false)
    }

    pub(super) fn room_close(&self, tab: Option<&str>, _task: Option<&str>, call: RoomClose) -> Result<String, String> {
        let tab = tab.filter(|id| !id.is_empty()).ok_or("only the Captain closes the room")?;
        if !self.is_room_captain(tab) {
            return Err("only the Captain closes the room".into());
        }
        let place = self.captain_place(tab, call.room.as_deref())?;
        let outcome = self.rooms.lock().unwrap().close(&place.id, call.reason.as_deref().unwrap_or(""), super::unix_millis())?;
        self.after_speech(&place.id, outcome, true)
    }

    /// A line typed into the room thread. It wakes every seat still working, and the Captain.
    pub(super) fn room_user_post(&self, tab: &str, text: &str) -> Result<(), String> {
        let id = self.tab(tab, |t| room::id_in(&t.env))?.ok_or_else(|| format!("no room on {tab}"))?;
        let outcome = self.rooms.lock().unwrap().post(&id, room::Speech::User { text: text.to_owned() }, super::unix_millis())?;
        self.after_speech(&id, outcome, true)?;
        Ok(())
    }

    pub(super) fn room_page(&self, tab: &str, from: u64, limit: Option<u32>, before: Option<u64>) -> Result<ThreadPage, String> {
        let id = self.tab(tab, |t| room::id_in(&t.env))?.ok_or_else(|| format!("no room on {tab}"))?;
        let room = self.rooms.lock().unwrap().get(&id).cloned().ok_or_else(|| format!("no room {id}"))?;
        let all = page_items(&room);
        let (items, start) = match limit {
            Some(limit) => {
                let before = before.unwrap_or(u64::MAX);
                let eligible: Vec<_> = all.into_iter().filter(|(seq, _)| *seq < before).collect();
                let keep = (limit as usize).clamp(1, 200);
                let start_at = eligible.len().saturating_sub(keep);
                let page = &eligible[start_at..];
                let start = if start_at == 0 { 0 } else { page.first().map(|(seq, _)| *seq).unwrap_or(0) };
                (page.iter().map(|(_, item)| item.clone()).collect(), Some(start))
            }
            None => (all.into_iter().filter(|(seq, _)| *seq > from || (*seq == 0 && from == 0)).map(|(_, item)| item).collect(), None),
        };
        Ok(ThreadPage { session: None, items, next: room.seq, reset: false, context: None, branch: None, todos: None, agents: vec![], start, opening: None })
    }

    /// A room thread carries `PINNED_ROOM` and no role. A worker also carries the variable, and a role.
    pub(super) fn is_room_tab(&self, tab: &str) -> bool {
        self.tab(tab, |t| room::id_in(&t.env).is_some() && role_of(&t.env).is_none()).unwrap_or(false)
    }

    /// A seated worker's tab remembers the room, so a restart still knows where they sit.
    pub(super) fn note_room_env(&self, tab: &str, task: &str) {
        let Some(id) = self.delegates.get(task).and_then(|info| info.client_request_id).as_deref().and_then(room::seat_of).map(|(id, _)| id.to_owned()) else { return };
        let _ = self.tab(tab, |t| {
            if !t.env.iter().any(|(key, _)| key == room::ENV) {
                t.env.push((room::ENV.to_owned(), id));
            }
        });
    }

    /// The seat's task is still the room's, and the seat has not finished. The turn does not end it.
    pub(super) fn room_keeps_seat(&self, task: &str) -> bool {
        let Some((id, name)) = self.seat_of_task(task) else { return false };
        let rooms = self.rooms.lock().unwrap();
        rooms.get(&id).is_some_and(|room| room.status == room::Status::Open && room.seats.iter().any(|seat| seat.name == name && !seat.done))
    }

    /// The seat's turn failed. Mark it done so the room can still finish, then the caller settles the task.
    pub(super) fn room_seat_failed(&self, task: &str, message: &str) {
        let Some((id, name)) = self.seat_of_task(task) else { return };
        let outcome = match self.rooms.lock().unwrap().fail_seat(&id, &name, message, super::unix_millis()) {
            Ok(outcome) => outcome,
            Err(_) => return,
        };
        let _ = self.after_speech(&id, outcome, false);
    }

    /// A restart gives every tab a new id. Recreate a room thread, retie the seats, then send what is still unread.
    pub(super) fn reopen_rooms(&self) {
        let ids: Vec<String> = {
            let rooms = self.rooms.lock().unwrap();
            rooms.published_ids().into_iter().filter(|id| rooms.get(id).is_some_and(|saved| saved.status == room::Status::Open)).collect()
        };
        for id in &ids {
            let Some(saved) = self.rooms.lock().unwrap().get(id).cloned() else { continue };
            if !Path::new(&saved.project).is_dir() {
                continue;
            }
            let tab = self.ensure_room_tab(&saved);
            let owner = self.owner_for(&saved);
            let seats = self.seat_binding(&saved);
            self.rooms.lock().unwrap().rebind(id, &owner, &tab, &seats);
        }
        for id in &ids {
            self.retire_finished_seats(id);
        }
        let pending = self.rooms.lock().unwrap().pending_wakes();
        let mut by_room: Vec<(String, room::Outcome)> = vec![];
        for (id, wake) in pending {
            match by_room.iter_mut().find(|(room_id, _)| room_id == &id) {
                Some((_, outcome)) => outcome.wakes.push(wake),
                None => by_room.push((id, room::Outcome { seq: wake.through, closed: false, wakes: vec![wake] })),
            }
        }
        for (id, outcome) in by_room {
            self.deliver_room(&id, outcome);
        }
    }

    /// A closed tab ends what it held. A seat's tab ends that seat. The Captain's tab settles the
    /// rooms it owns and stops their seats. Called once the tab is gone, so no notice goes to it.
    pub(super) fn room_tab_closed(&self, tab: &str, task: Option<&str>) {
        if let Some((id, name)) = task.and_then(|task| self.seat_of_task(task)) {
            let outcome = self.rooms.lock().unwrap().seat_gone(&id, &name, super::unix_millis());
            if let Ok(outcome) = outcome {
                self.deliver_room(&id, outcome);
            }
        }
        let owned: Vec<String> = {
            let rooms = self.rooms.lock().unwrap();
            rooms.published_ids().into_iter().filter(|id| rooms.get(id).is_some_and(|saved| saved.owner_tab == tab && saved.status == room::Status::Open)).collect()
        };
        for id in owned {
            let outcome = self.rooms.lock().unwrap().close(&id, "its Captain's tab was closed", super::unix_millis());
            if let Ok(outcome) = outcome {
                let _ = self.after_speech(&id, outcome, true);
            }
        }
    }

    /// A notice queued before a crash must not be sent again. Nothing is queued here: tab ids are still the old ones.
    pub(super) fn reconcile_rooms(&self) {
        let keys = self.rooms.lock().unwrap().unacked_keys();
        let taken: Vec<String> = keys.into_iter().filter(|key| self.delegates.took_notice(key)).collect();
        if !taken.is_empty() {
            self.rooms.lock().unwrap().ack_taken(&taken);
        }
    }

    fn after_speech(&self, id: &str, outcome: room::Outcome, stop_when_closed: bool) -> Result<String, String> {
        let (seq, closed) = (outcome.seq, outcome.closed);
        self.deliver_room(id, outcome);
        if closed && stop_when_closed {
            self.stop_seats(id);
        }
        serde_json::to_string(&json!({ "seq": seq, "id": id, "closed": closed })).map_err(|e| e.to_string())
    }

    fn deliver_room(&self, id: &str, outcome: room::Outcome) {
        let mut follow = vec![];
        for wake in outcome.wakes {
            // No tab can carry it yet. It stays owed until `reopen_rooms` binds one.
            if wake.tab.is_empty() || !self.tabs.lock().unwrap().contains_key(&wake.tab) {
                continue;
            }
            if self.is_room_tab(&wake.tab) {
                self.rooms.lock().unwrap().ack(id, &wake.who, wake.through);
                continue;
            }
            if let room::Who::Seat(name) = &wake.who {
                if self.seat_task_terminal(id, name) {
                    self.rooms.lock().unwrap().ack(id, &wake.who, wake.through);
                    if let Ok(next) = self.rooms.lock().unwrap().seat_gone(id, name, super::unix_millis()) {
                        if !next.wakes.is_empty() {
                            follow.push(next);
                        }
                    }
                    continue;
                }
            }
            let session = self.tab(&wake.tab, |t| t.claude_session.clone()).ok().flatten();
            let queued = self.delegates.queue_notice(&wake.key, &wake.tab, session, wake.text);
            if queued || self.delegates.took_notice(&wake.key) {
                self.rooms.lock().unwrap().ack(id, &wake.who, wake.through);
            }
            if queued {
                self.maybe_deliver(&wake.tab);
            }
        }
        for outcome in follow {
            self.deliver_room(id, outcome);
        }
        if let Some(tab) = self.rooms.lock().unwrap().get(id).map(|saved| saved.tab.clone()).filter(|tab| !tab.is_empty()) {
            self.emit(super::Event::Thread { tab });
        }
    }

    fn stop_seats(&self, id: &str) {
        let (owner, tasks) = {
            let rooms = self.rooms.lock().unwrap();
            let Some(saved) = rooms.get(id) else { return };
            (saved.owner_tab.clone(), rooms.seat_tasks(id))
        };
        for task in tasks {
            if self.delegates.get(&task).is_some_and(|info| !info.state.terminal()) {
                let _ = self.task_cancel(Some(owner.clone()), None, &task, "the room closed");
            }
        }
    }

    fn captain_project(&self, tab: &str) -> Result<PathBuf, String> {
        let (env, task, cwd) = self.tab(tab, |t| (t.env.clone(), t.task.clone(), t.cwd.clone()))?;
        if crate::global_orchestrator::is_global(&env) {
            return Err(super::NO_PROJECT_WORK.into());
        }
        match (role_of(&env), task) {
            (Some((Role::Orchestrator, _)), None) => Ok(history::project_of(&cwd)),
            _ => Err("only a project's Captain opens a room".into()),
        }
    }

    fn is_room_captain(&self, tab: &str) -> bool {
        self.captain_project(tab).is_ok()
    }

    fn place(&self, tab: Option<&str>, task: Option<&str>, asked: Option<&str>) -> Result<Place, String> {
        if let Some((id, name)) = task.filter(|id| !id.is_empty()).and_then(|id| self.seat_of_task(id)) {
            return self.seat_place(&id, &name, asked);
        }
        if let Some(tab) = tab.filter(|id| !id.is_empty()) {
            if let Some(task) = self.tab(tab, |t| t.task.clone()).ok().flatten() {
                if let Some((id, name)) = self.seat_of_task(&task) {
                    return self.seat_place(&id, &name, asked);
                }
            }
            if self.is_room_captain(tab) {
                return self.captain_place(tab, asked);
            }
        }
        Err("you are not seated in a room".into())
    }

    fn seat_place(&self, id: &str, name: &str, asked: Option<&str>) -> Result<Place, String> {
        if let Some(asked) = asked.map(str::trim).filter(|id| !id.is_empty()) {
            if asked != id {
                return Err(format!("you are seated in {id}, not {asked}"));
            }
        }
        if self.rooms.lock().unwrap().get(id).is_none() {
            return Err(format!("no room {id}"));
        }
        Ok(Place { id: id.to_owned(), seat: Some(name.to_owned()) })
    }

    fn captain_place(&self, tab: &str, asked: Option<&str>) -> Result<Place, String> {
        let rooms = self.rooms.lock().unwrap();
        if let Some(asked) = asked.map(str::trim).filter(|id| !id.is_empty()) {
            let Some(saved) = rooms.get(asked).filter(|saved| saved.published) else {
                return Err(format!("no room {asked}"));
            };
            if saved.owner_tab != tab {
                return Err(format!("{asked} belongs to another Captain"));
            }
            return Ok(Place { id: asked.to_owned(), seat: None });
        }
        let ids = rooms.published_ids();
        let owned: Vec<String> = ids.into_iter().filter(|id| rooms.get(id).is_some_and(|saved| saved.owner_tab == tab)).collect();
        match owned.len() {
            1 => Ok(Place { id: owned[0].clone(), seat: None }),
            0 => Err("you are not seated in a room".into()),
            _ => Err(format!("name the room: {}", owned.join(", "))),
        }
    }

    fn seat_of_task(&self, task: &str) -> Option<(String, String)> {
        self.delegates.get(task).and_then(|info| info.client_request_id).as_deref().and_then(room::seat_of).map(|(id, name)| (id.to_owned(), name.to_owned()))
    }

    fn seat_task_terminal(&self, id: &str, name: &str) -> bool {
        let task = self.rooms.lock().unwrap().get(id).and_then(|saved| saved.seats.iter().find(|seat| seat.name == name).map(|seat| seat.task.clone()));
        task.is_some_and(|task| !task.is_empty() && self.delegates.get(&task).is_some_and(|info| info.state.terminal()))
    }

    fn add_room_tab(&self, captain: &str, id: &str, brief: &str, project: &Path) -> Result<String, String> {
        let (cwd, account) = self.tab(captain, |t| (t.cwd.clone(), t.account.clone()))?;
        let env = vec![(PROJECT_ENV.to_owned(), project.display().to_string()), (room::ENV.to_owned(), id.to_owned())];
        let tab = self.add_tab(cwd, account, vec![], env, false, None);
        let title = crate::pinned_lines::flat(brief, 80);
        self.tab(&tab, |t| {
            t.asleep = true;
            t.spawned_by = Some(captain.to_owned());
            t.title = Some(title);
        })?;
        Ok(tab)
    }

    fn start_seat(&self, captain: &str, id: &str, brief: &str, name: &str, names: &[String], seat: &RoomSeatSpec, isolation: &str) -> Result<(String, String), String> {
        let others: Vec<String> = names.iter().filter(|other| other.as_str() != name).cloned().collect();
        let mut target = json!({ "machine": crate::placement::HERE });
        if let Some(model) = seat.model.as_ref().filter(|model| !model.is_empty()) {
            target["model"] = json!(model);
        }
        if let Some(effort) = seat.effort.as_ref().filter(|effort| !effort.is_empty()) {
            target["options"] = json!({ "effort": effort });
        }
        let mut args = json!({
            "task": room::seat_brief(id, name, &others, brief),
            "title": name,
            "isolation": isolation,
            "mode": "async",
            "clientRequestId": room::request_id(id, name),
            "target": target,
        });
        if !seat.tools.is_empty() {
            args["tools"] = json!(seat.tools);
        }
        let request = crate::delegate::parse_request(&args)?;
        let body = self.start_delegate(Some(captain.to_owned()), None, Some("room_open".into()), request)?;
        let started: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let task = started["taskId"].as_str().filter(|id| !id.is_empty()).ok_or_else(|| format!("{name} did not start"))?.to_owned();
        let tab = started["tabId"].as_str().filter(|id| !id.is_empty()).map(str::to_owned).or_else(|| self.delegates.get(&task).and_then(|info| info.thread.map(|thread| thread.tab))).ok_or_else(|| format!("{name} has no thread"))?;
        Ok((task, tab))
    }

    fn rollback_room(&self, captain: &str, id: &str, room_tab: &str, started: &[(String, String, String)]) {
        for (_, _, task) in started {
            let _ = self.task_cancel(Some(captain.to_owned()), None, task, "the room did not open");
        }
        self.close_tab(room_tab);
        self.rooms.lock().unwrap().discard(id);
    }

    fn ensure_room_tab(&self, saved: &room::Room) -> String {
        let found = self.tabs.lock().unwrap().iter().find(|(_, t)| room::id_in(&t.env).as_deref() == Some(saved.id.as_str()) && role_of(&t.env).is_none()).map(|(id, _)| id.clone());
        if let Some(id) = found {
            return id;
        }
        let project = PathBuf::from(&saved.project);
        // `account_for` locks the project list. Take that after the tab lock is dropped.
        let from_open = {
            let tabs = self.tabs.lock().unwrap();
            tabs.iter()
                .find(|(id, tab)| id.as_str() == saved.owner_tab || role_of(&tab.env) == Some((Role::Orchestrator, project.clone())))
                .map(|(id, t)| (t.cwd.clone(), t.account.clone(), Some(id.clone())))
        };
        let (cwd, account, spawned_by) = match from_open {
            Some(found) => found,
            None => (project.clone(), self.account_for(None, &project).unwrap_or_else(|_| crate::accounts::Account::builtin(crate::accounts::Provider::Claude)), None),
        };
        let env = vec![(PROJECT_ENV.to_owned(), project.display().to_string()), (room::ENV.to_owned(), saved.id.clone())];
        let tab = self.add_tab(cwd, account, vec![], env, false, None);
        let title = crate::pinned_lines::flat(&saved.brief, 80);
        let _ = self.tab(&tab, |t| {
            t.asleep = true;
            t.spawned_by = spawned_by;
            t.title = Some(title);
        });
        tab
    }

    fn owner_for(&self, saved: &room::Room) -> String {
        if self.tabs.lock().unwrap().contains_key(&saved.owner_tab) {
            return saved.owner_tab.clone();
        }
        let project = PathBuf::from(&saved.project);
        self.tabs
            .lock()
            .unwrap()
            .iter()
            .find(|(_, t)| role_of(&t.env) == Some((Role::Orchestrator, project.clone())) && !crate::global_orchestrator::is_global(&t.env))
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| saved.owner_tab.clone())
    }

    fn seat_binding(&self, saved: &room::Room) -> Vec<(String, String, String)> {
        saved
            .seats
            .iter()
            .map(|seat| {
                let from_task = self.delegates.get(&seat.task).and_then(|info| info.thread.map(|thread| thread.tab)).filter(|tab| self.tabs.lock().unwrap().contains_key(tab));
                let tab = from_task
                    .or_else(|| self.tabs.lock().unwrap().iter().find(|(_, t)| t.task.as_deref() == Some(seat.task.as_str())).map(|(id, _)| id.clone()))
                    .unwrap_or_else(|| seat.tab.clone());
                (seat.name.clone(), seat.task.clone(), tab)
            })
            .collect()
    }

    fn retire_finished_seats(&self, id: &str) {
        let seats = self.rooms.lock().unwrap().get(id).map(|saved| saved.seats.iter().filter(|seat| !seat.done).map(|seat| (seat.name.clone(), seat.task.clone())).collect::<Vec<_>>()).unwrap_or_default();
        for (name, task) in seats {
            let gone = task.is_empty() || self.delegates.get(&task).map(|info| info.state.terminal()).unwrap_or(true);
            if !gone {
                continue;
            }
            let outcome = self.rooms.lock().unwrap().seat_gone(id, &name, super::unix_millis());
            if let Ok(outcome) = outcome {
                self.deliver_room(id, outcome);
            }
        }
    }
}

fn isolation_of(raw: Option<&str>) -> Result<&'static str, String> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("shared") => Ok("shared"),
        Some("worktree") => Ok("worktree"),
        Some(other) => Err(format!("isolation must be shared or worktree, not {other:?}")),
    }
}

fn seats_do_not_fit(seats: usize, parallel: u32) -> String {
    format!("a room seats {seats} workers and this project allows {parallel} at once. Seat fewer, or raise Workers at once in Settings › Way of working.")
}

fn stamp(at: u64) -> String {
    chrono::DateTime::from_timestamp_millis(at as i64).map(|time| time.to_rfc3339()).unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

fn page_items(saved: &room::Room) -> Vec<(u64, thread::Item)> {
    let mut items = vec![(0, thread::Item::Note { id: format!("{}-brief", saved.id), at: stamp(saved.opened), text: saved.brief.clone() })];
    for post in &saved.posts {
        let at = stamp(post.at);
        let id = post.seq.to_string();
        let item = match &post.author {
            room::Author::User | room::Author::Card => thread::Item::User { id, at, text: room::line_of(post), images: 0, pictures: vec![] },
            room::Author::Seat { .. } | room::Author::System => thread::Item::Text { id, at, text: room::line_of(post) },
        };
        items.push((post.seq, item));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::super::tests::launch;
    use super::*;
    use crate::accounts::Provider;
    use crate::agent::{AgentEvent, AgentStatus};
    use crate::taste::{TastePatch, TasteScope, Tier};
    use crate::wire::Call;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::Path;

    #[cfg(unix)]
    fn thread_claude(dir: &Path) -> PathBuf {
        // A turn's Done prices its tokens, and the price download needs the TLS provider `main` installs.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let claude = dir.join("claude");
        let script = "#!/bin/sh\nif [ \"$1 $2\" = \"auth status\" ]; then echo '{\"loggedIn\":true,\"email\":\"ana@acme.dev\"}'; exit 0; fi\nprintf '%s\\0' \"$(pwd -P)\" \"$PINNED_ROLE\" \"$PINNED_PROJECT\" \"$CLAUDE_CONFIG_DIR\" \"$@\" > \"$0.tmp.$PINNED_TAB\" && mv \"$0.tmp.$PINNED_TAB\" \"$0.$PINNED_TAB\"\nexec sleep 30\n";
        std::fs::write(&claude, script).unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        claude
    }

    #[test]
    #[cfg(unix)]
    fn a_room_opens_seats_wakes_them_and_closes() {
        let dir = std::env::temp_dir().join(format!("pinned-room-it-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let project = dir.join("app");
        let m = Machine::new(super::super::Launch { bins: BTreeMap::from([(Provider::Claude, thread_claude(&dir))]), ..launch() }, Some(dir.join("config")), None, |_| {});
        m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }).unwrap();
        m.set_local_taste(&TasteScope::Captains, TastePatch { tier: Some(Tier::High), parallel: Some(2), ..TastePatch::default() }).unwrap();
        let path = m.open_project(project).display().to_string();
        let captain = m.open_orchestrator(&path).unwrap();
        let open = |brief: &str| {
            let tool = crate::team_tools::request("room_open", &json!({ "brief": brief, "seats": [{ "name": "Ana" }, { "name": "Beto" }] })).unwrap();
            m.team_tool(Some(&captain), None, tool)
        };
        let body: Value = serde_json::from_str(&open("Fix the parser.").unwrap()).unwrap();
        let id = body["id"].as_str().unwrap().to_owned();
        let room_tab = body["tabId"].as_str().unwrap().to_owned();
        let seat = |name: &str| {
            let row = body["seats"].as_array().unwrap().iter().find(|row| row["name"] == name).unwrap();
            (row["task"].as_str().unwrap().to_owned(), row["tab"].as_str().unwrap().to_owned())
        };
        let (ana_task, ana_tab) = seat("Ana");
        let (beto_task, beto_tab) = seat("Beto");
        assert!(!m.delegates.get(&ana_task).unwrap().state.terminal());
        assert!(!m.delegates.get(&beto_task).unwrap().state.terminal());
        assert_eq!(m.delegates.working_in(Path::new(&path)), 2);
        assert!(m.tab(&room_tab, |t| t.asleep && room::id_in(&t.env).as_deref() == Some(id.as_str()) && role_of(&t.env).is_none()).unwrap());
        assert!(m.view(&room_tab).unwrap().running);
        assert_eq!(m.tab(&ana_tab, |t| room::id_in(&t.env)).unwrap().as_deref(), Some(id.as_str()));
        let page = m.thread_page(&room_tab, 0, None, None).unwrap();
        assert!(page.items.iter().any(|item| matches!(item, thread::Item::Note { text, .. } if text.contains("Fix the parser"))));

        let refused = open("Another room.").unwrap_err();
        assert!(refused.contains("already run"), "{refused}");
        assert_eq!(m.delegates.working_in(Path::new(&path)), 2);
        assert_eq!(m.tabs.lock().unwrap().values().filter(|t| room::id_in(&t.env).is_some() && role_of(&t.env).is_none()).count(), 1);
        assert!(m.rooms.lock().unwrap().get("room-2").is_none());

        let post = crate::team_tools::request("room_post", &json!({ "text": "the split is the colon" })).unwrap();
        m.team_tool(Some(&ana_tab), Some(&ana_task), post).unwrap();
        assert_eq!(m.delegates.queued().get(&beto_tab).copied().unwrap_or(0), 1);
        assert_eq!(m.delegates.queued().get(&captain).copied().unwrap_or(0), 0);

        assert_eq!(m.call(Call::Prompt { tab: room_tab.clone(), text: "Ship it.".into() }), Ok(json!(null)));
        let page = m.thread_page(&room_tab, 0, None, None).unwrap();
        assert!(page.items.iter().any(|item| matches!(item, thread::Item::User { text, .. } if text.contains("You: Ship it."))));

        let done_turn = |reply: &str| {
            m.record_event(AgentEvent {
                status: AgentStatus::Done,
                cwd: None,
                message: None,
                tab: Some(ana_tab.clone()),
                session: None,
                title: None,
                resume_at: None,
                prompt: None,
                snapshot: None,
                permission_mode: None,
                hook_feedback: false,
                reply: Some(reply.into()),
            });
        };
        done_turn("still working");
        assert!(!m.delegates.get(&ana_task).unwrap().state.terminal(), "a room seat stays up after its first turn");

        let before = m.delegates.queued().get(&captain).copied().unwrap_or(0);
        let finished = crate::team_tools::request("room_done", &json!({ "summary": "parser splits" })).unwrap();
        let done: Value = serde_json::from_str(&m.team_tool(Some(&ana_tab), Some(&ana_task), finished).unwrap()).unwrap();
        assert_eq!(done["closed"], false);
        assert!(m.delegates.queued().get(&captain).copied().unwrap_or(0) > before);
        assert!(!m.delegates.get(&ana_task).unwrap().state.terminal(), "room_done leaves the task until its turn ends");
        done_turn("shipped");
        assert!(m.delegates.get(&ana_task).unwrap().state.terminal(), "the Done after room_done settles the seat");

        let close = crate::team_tools::request("room_close", &json!({ "reason": "enough" })).unwrap();
        let closed: Value = serde_json::from_str(&m.team_tool(Some(&captain), None, close).unwrap()).unwrap();
        assert_eq!(closed["closed"], true);
        assert!(m.delegates.get(&beto_task).unwrap().state.terminal());
        assert_eq!(m.delegates.working_in(Path::new(&path)), 0);
        let read: Value = serde_json::from_str(&m.team_tool(Some(&captain), None, crate::team_tools::request("room_read", &json!({})).unwrap()).unwrap()).unwrap();
        assert_eq!(read["status"], "settled");
        assert!(read["posts"].as_array().unwrap().iter().any(|post| post["text"].as_str().unwrap_or("").contains("Ship it")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn machine_on(dir: &Path) -> std::sync::Arc<Machine> {
        let m = Machine::new(super::super::Launch { bins: BTreeMap::from([(Provider::Claude, thread_claude(dir))]), ..launch() }, Some(dir.join("config")), None, |_| {});
        // A restart is a second machine over the same config, which already holds the account.
        if let Err(e) = m.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into(), auth: None, key: None }) {
            assert_eq!(e, "an account work already exists");
        }
        m.set_local_taste(&TasteScope::Captains, TastePatch { tier: Some(Tier::High), parallel: Some(2), ..TastePatch::default() }).unwrap();
        m
    }

    #[cfg(unix)]
    fn captain_on(m: &Machine, dir: &Path, app: &str) -> (String, String) {
        let path = m.open_project(dir.join(app)).display().to_string();
        let captain = m.open_orchestrator(&path).unwrap();
        (captain, path)
    }

    #[cfg(unix)]
    fn open_room(m: &Machine, captain: &str, brief: &str) -> Value {
        let tool = crate::team_tools::request("room_open", &json!({ "brief": brief, "seats": [{ "name": "Ana" }, { "name": "Beto" }] })).unwrap();
        serde_json::from_str(&m.team_tool(Some(captain), None, tool).unwrap()).unwrap()
    }

    #[cfg(unix)]
    fn seat_of(body: &Value, name: &str) -> (String, String) {
        let row = body["seats"].as_array().unwrap().iter().find(|row| row["name"] == name).unwrap();
        (row["task"].as_str().unwrap().to_owned(), row["tab"].as_str().unwrap().to_owned())
    }

    #[cfg(unix)]
    fn posted(m: &Machine, tab: &str, task: &str, text: &str, to: Value) -> Result<String, String> {
        let tool = crate::team_tools::request("room_post", &json!({ "text": text, "to": to })).unwrap();
        m.team_tool(Some(tab), Some(task), tool)
    }

    #[test]
    #[cfg(unix)]
    fn closing_a_tab_ends_the_room_it_owns_or_the_seat_it_holds() {
        let dir = std::env::temp_dir().join(format!("pinned-room-close-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let m = machine_on(&dir);
        let (captain, _path) = captain_on(&m, &dir, "app");
        let body = open_room(&m, &captain, "Fix the parser.");
        let id = body["id"].as_str().unwrap().to_owned();
        let (ana_task, ana_tab) = seat_of(&body, "Ana");
        let (beto_task, beto_tab) = seat_of(&body, "Beto");

        m.close_tab(&ana_tab);
        let room = m.rooms.lock().unwrap().get(&id).cloned().unwrap();
        assert!(room.seats.iter().find(|seat| seat.name == "Ana").unwrap().done, "closing a seat's tab ends that seat");
        assert_eq!(room.status, room::Status::Open, "the room lives on");
        assert!(!m.delegates.get(&beto_task).unwrap().state.terminal(), "the other seat keeps working");
        let still = posted(&m, &beto_tab, &beto_task, "I will take the tests", json!([])).unwrap();
        assert!(still.contains("\"closed\":false"), "{still}");
        assert_eq!(posted(&m, &ana_tab, &ana_task, "still here", json!([])).unwrap_err(), "Ana already finished");

        m.close_tab(&captain);
        let room = m.rooms.lock().unwrap().get(&id).cloned().unwrap();
        assert_eq!(room.status, room::Status::Settled, "closing the Captain's tab settles the room");
        assert!(m.delegates.get(&beto_task).unwrap().state.terminal(), "the running seats are stopped");
        assert_eq!(posted(&m, &beto_tab, &beto_task, "more", json!([])).unwrap_err(), "room-1 is closed");
        let read = m.rooms.lock().unwrap().read(&id, 0).unwrap();
        assert!(read["posts"].as_array().unwrap().iter().any(|post| post["text"].as_str().unwrap_or("").contains("take the tests")), "the messages stay readable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_restart_reopens_only_open_rooms() {
        let dir = std::env::temp_dir().join(format!("pinned-room-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("one")).unwrap();
        std::fs::create_dir_all(dir.join("two")).unwrap();
        let m = machine_on(&dir);
        let (captain_one, _) = captain_on(&m, &dir, "one");
        let (captain_two, _) = captain_on(&m, &dir, "two");
        let kept = open_room(&m, &captain_one, "Fix the parser.");
        let kept_id = kept["id"].as_str().unwrap().to_owned();
        let gone = open_room(&m, &captain_two, "Write the docs.");
        let gone_id = gone["id"].as_str().unwrap().to_owned();
        m.close_tab(&captain_two);
        let gone_seq = m.rooms.lock().unwrap().get(&gone_id).unwrap().seq;
        assert_eq!(m.rooms.lock().unwrap().get(&gone_id).unwrap().status, room::Status::Settled, "the closed tab settles its room");

        let again = machine_on(&dir);
        again.restore_tabs();
        let tabs_of = |id: &str| again.tabs.lock().unwrap().iter().filter(|(_, t)| room::id_in(&t.env).as_deref() == Some(id) && role_of(&t.env).is_none()).count();
        assert_eq!(tabs_of(&kept_id), 1, "the open room's thread comes back");
        assert_eq!(tabs_of(&gone_id), 0, "a settled room is not resurrected");
        assert_eq!(again.rooms.lock().unwrap().get(&gone_id).unwrap().status, room::Status::Settled, "it loads back settled");
        assert!(again.rooms.lock().unwrap().pending_wakes().iter().all(|(id, _)| id != &gone_id), "a settled room owes no wake");
        assert!(!again.delegates.took_notice(&room::wake_key(&gone_id, room::CAPTAIN, gone_seq)), "its close line is not sent again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_wake_that_waited_for_a_seat_arrives_when_the_seat_is_bound() {
        let dir = std::env::temp_dir().join(format!("pinned-room-bind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let m = machine_on(&dir);
        let (captain, _path) = captain_on(&m, &dir, "app");
        let body = open_room(&m, &captain, "Fix the parser.");
        let id = body["id"].as_str().unwrap().to_owned();
        let (ana_task, ana_tab) = seat_of(&body, "Ana");
        let (beto_task, beto_tab) = seat_of(&body, "Beto");

        // the window `room_open` leaves while a later seat still has no tab
        m.rooms.lock().unwrap().bind_seat(&id, "Beto", &beto_task, "").unwrap();
        posted(&m, &ana_tab, &ana_task, "Beto, take the tests", json!(["Beto"])).unwrap();
        assert_eq!(m.delegates.queued().get(&beto_tab).copied().unwrap_or(0), 0, "nothing can carry it yet");

        // a restart's retie is what binds the seat back, and it sends what waited
        m.rooms.lock().unwrap().bind_seat(&id, "Beto", &beto_task, &beto_tab).unwrap();
        m.reopen_rooms();
        assert_eq!(m.delegates.queued().get(&beto_tab).copied().unwrap_or(0), 1, "the waiting post arrives once the seat is bound");
        assert!(!m.rooms.lock().unwrap().unacked_keys().contains(&room::wake_key(&id, "Beto", 1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_wake_for_a_captain_tab_that_left_waits_for_the_captain_to_return() {
        let dir = std::env::temp_dir().join(format!("pinned-room-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let m = machine_on(&dir);
        let (captain, path) = captain_on(&m, &dir, "app");
        let body = open_room(&m, &captain, "Fix the parser.");
        let id = body["id"].as_str().unwrap().to_owned();
        let (ana_task, ana_tab) = seat_of(&body, "Ana");

        // a restart clears the tab map without settling anything, as `quit` does
        m.tabs.lock().unwrap().remove(&captain).unwrap();
        posted(&m, &ana_tab, &ana_task, "Captain, the split is wrong", json!(["captain"])).unwrap();
        let key = room::wake_key(&id, room::CAPTAIN, 1);
        assert!(!m.delegates.took_notice(&key), "no tab can carry it, so nothing is queued in its name");
        assert!(m.rooms.lock().unwrap().unacked_keys().contains(&key), "the wake stays owed");

        // the Captain returns on a new tab and the rooms reopen onto it
        let back = m.open_orchestrator(&path).unwrap();
        m.reopen_rooms();
        assert_eq!(m.rooms.lock().unwrap().get(&id).unwrap().owner_tab, back);
        assert!(m.delegates.queued().get(&back).copied().unwrap_or(0) > 0, "the waiting post reaches the Captain's new tab");
        assert!(!m.rooms.lock().unwrap().unacked_keys().contains(&key));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_repeated_room_post_id_answers_the_first_and_appends_nothing() {
        let dir = std::env::temp_dir().join(format!("pinned-room-idem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let m = machine_on(&dir);
        let (captain, _path) = captain_on(&m, &dir, "app");
        let body = open_room(&m, &captain, "Fix the parser.");
        let (ana_task, ana_tab) = seat_of(&body, "Ana");
        let say = |text: &str, id: Option<&str>| {
            let mut args = json!({ "text": text });
            if let Some(id) = id {
                args["id"] = json!(id);
            }
            let tool = crate::team_tools::request("room_post", &args).unwrap();
            m.team_tool(Some(&ana_tab), Some(&ana_task), tool)
        };

        let first: Value = serde_json::from_str(&say("the split is the colon", Some("say-1")).unwrap()).unwrap();
        let again: Value = serde_json::from_str(&say("the split is the colon", Some("say-1")).unwrap()).unwrap();
        assert_eq!(again, first, "the repeat answers the first outcome");
        let late = say("a different text under the same id", Some("say-1")).unwrap();
        assert_eq!(late, serde_json::to_string(&first).unwrap(), "even different text under the same id appends nothing");

        let read: Value = serde_json::from_str(&m.team_tool(Some(&captain), None, crate::team_tools::request("room_read", &json!({})).unwrap()).unwrap()).unwrap();
        let texts: Vec<&str> = read["posts"].as_array().unwrap().iter().map(|post| post["text"].as_str().unwrap()).collect();
        assert_eq!(texts, vec!["the split is the colon"], "one message");
        let next: Value = serde_json::from_str(&say("and the tests", Some("say-2")).unwrap()).unwrap();
        assert_eq!(next["seq"], 2, "a fresh id posts");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
