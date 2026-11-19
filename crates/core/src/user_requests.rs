//! The global Orchestrator's user requests: one record per thread the user opened, each with its
//! own Orchestrator session, kept in `user-requests.json` in the orchestration folder with every
//! request id a global session sent, so a Captain's report finds the thread that asked. Times are
//! Unix milliseconds.

use crate::plan::{MAX_NAME, MAX_TITLE, MAX_WHAT};
use crate::wire::{MachineId, TabId};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::path::{Path, PathBuf};

pub const FILE: &str = "user-requests.json";
/// As many requests as the Orchestrator's ledger keeps messages. Done ones fall off first, oldest
/// first; open ones never do.
pub const KEEP: usize = 500;
pub const MAX_CAPTAINS: usize = 8;
/// The longest Captain folder, in bytes, a request records.
pub const MAX_PATH: usize = 4096;
pub const MAX_LINKS: usize = 6;
pub const MAX_LAST: usize = 120;
pub const MAX_GLANCES: usize = 60;
/// How long a Done request stays in the glances.
pub const GLANCE_DONE_MS: u64 = 14 * 24 * 60 * 60 * 1000;
pub const DIGEST_LINES: usize = 12;
pub const DIGEST_TEXT: usize = 120;
pub const EARLIER_TITLE: &str = "Earlier conversation";
/// The environment variable that marks a global tab as request `<id>`'s own.
pub const ENV: &str = "PINNED_REQUEST";
/// Sent ids and deliveries kept; a report answering an id that fell off takes the fallback.
const SENT_KEEP: usize = 2000;
const DELIVERED_KEEP: usize = 2000;
const ID_LETTERS: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const ID_LEN: usize = 10;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UserRequest {
    /// `q` and 10 random base32 letters, never reused. Also the prefix of the request ids its
    /// session sends.
    pub id: String,
    /// The app's idempotency key: opening the same key again answers this request.
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub project: Option<String>,
    pub opened_at: u64,
    /// Its Orchestrator Claude session. The tab is found from it at runtime.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub stands: Stands,
    #[serde(default)]
    pub closed: Option<Closed>,
    /// The Captains its session sent to, newest last.
    #[serde(default)]
    pub captains: Vec<CaptainRef>,
    #[serde(default)]
    pub last: Option<Line>,
    #[serde(default)]
    pub last_user_at: Option<u64>,
    #[serde(default)]
    pub replies: u32,
    #[serde(default)]
    pub kind: Kind,
    /// Reports held for its first session while the cap kept it from starting, kept here so a
    /// restart still types them. Cleared once its session starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<String>,
}

/// "Where this stands": each text one line of at most 300 characters.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Stands {
    #[serde(default)]
    pub asked: String,
    #[serde(default)]
    pub done: String,
    #[serde(default)]
    pub left: String,
    /// What waits on the user; empty when nothing does.
    #[serde(default)]
    pub needs: String,
    #[serde(default)]
    pub needs_at: Option<u64>,
    /// At most 6, each one line of at most 60 characters, plain text.
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub at: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Closed {
    pub by: ClosedBy,
    pub at: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClosedBy {
    User,
    Orchestrator,
}

/// A Captain a request sent to, or the one that reported.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptainRef {
    #[serde(default)]
    pub machine: MachineId,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub machine_name: String,
    /// Sent with `box`: the box and its folder may change, so it is matched by machine or repo.
    #[serde(default)]
    pub boxed: bool,
    #[serde(default)]
    pub at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub who: Who,
    pub text: String,
    pub at: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    User,
    Orchestrator,
    Captain,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The user opened it.
    #[default]
    Asked,
    /// "Reports from Captain X": reports no request asked for.
    CaptainReports,
    /// The single Orchestrator conversation from before threads.
    Earlier,
}

/// What the app lists for a request.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct RequestGlance {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub project: Option<String>,
    pub opened_at: u64,
    #[serde(default)]
    pub kind: Kind,
    /// Done when set.
    #[serde(default)]
    pub closed_at: Option<u64>,
    /// Empty when nothing waits on the user.
    #[serde(default)]
    pub needs: String,
    #[serde(default)]
    pub needs_at: Option<u64>,
    #[serde(default)]
    pub last_user_at: Option<u64>,
    /// The thread's last line, flat: no struct sits inside an Option on the wire.
    #[serde(default)]
    pub last_who: Option<Who>,
    #[serde(default)]
    pub last_text: String,
    #[serde(default)]
    pub last_at: Option<u64>,
    /// Orchestrator replies and reports in the thread.
    #[serde(default)]
    pub replies: u32,
    #[serde(default)]
    pub captains: Vec<GlanceCaptain>,
    #[serde(default)]
    pub tab: Option<TabId>,
    /// What the user first asked, one line of at most 300 characters.
    #[serde(default)]
    pub asked: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct GlanceCaptain {
    #[serde(default)]
    pub machine: MachineId,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub machine_name: String,
}

/// Where a report goes: a request, or a global session no request holds, such as the anchor
/// before migration. An empty session is the anchor that sent before its session started.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "to", content = "id", rename_all = "snake_case")]
pub enum Route {
    Request(String),
    Session(String),
}

/// A `request_update` call. A field left out stays as it is.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct StandsPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub asked: Option<String>,
    #[serde(default)]
    pub done: Option<String>,
    #[serde(default)]
    pub left: Option<String>,
    #[serde(default)]
    pub needs: Option<String>,
    #[serde(default)]
    pub links: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct SentId {
    id: String,
    /// The global session that sent it; empty when the tab's session had not started.
    #[serde(default)]
    session: String,
    /// The request whose tab sent it. A request keeps its id when its session changes, as after
    /// `/clear`, so a report routes by it first.
    #[serde(default)]
    request: Option<String>,
    at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Delivered {
    report: String,
    to: Route,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    requests: Vec<UserRequest>,
    /// Every request id a global session sent, oldest first.
    #[serde(default)]
    sent: Vec<SentId>,
    /// Each (report, route) already typed, so a retry types only what is left.
    #[serde(default)]
    delivered: Vec<Delivered>,
    /// The old single conversation became a request, or there was none.
    #[serde(default)]
    migrated: bool,
}

#[derive(Default)]
pub struct UserRequests {
    saved: Saved,
    /// None keeps the requests in memory only.
    file: Option<PathBuf>,
}

impl UserRequests {
    /// The requests saved in `dir`, or none when the file is missing. A file that can't be read or
    /// parsed is renamed to `user-requests.json.bad` before anything is saved over it; when even
    /// that fails, the requests stay in memory only.
    pub fn load(dir: Option<&Path>) -> Self {
        let mut file = dir.map(|d| d.join(FILE));
        let mut saved = Saved::default();
        if let Some(path) = &file {
            let read = match std::fs::read(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Saved::default()),
                Err(e) => Err(e.to_string()),
                // serde's message may quote the file, and a request's text stays out of the log.
                Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("no JSON requests at line {} column {}", e.line(), e.column())),
            };
            match read {
                Ok(read) => saved = read,
                Err(e) => {
                    let bad = path.with_extension("json.bad");
                    eprintln!("pinned: {FILE} is unreadable ({e}); kept aside as {FILE}.bad");
                    if let Err(e) = std::fs::rename(path, &bad) {
                        eprintln!("pinned: {FILE} could not be kept aside ({e}); requests will not save");
                        file = None;
                    }
                }
            }
        }
        UserRequests { saved, file }
    }

    pub fn all(&self) -> &[UserRequest] {
        &self.saved.requests
    }

    pub fn get(&self, id: &str) -> Option<&UserRequest> {
        self.saved.requests.iter().find(|r| r.id == id)
    }

    pub fn of_session(&self, session: &str) -> Option<&UserRequest> {
        self.saved.requests.iter().find(|r| r.session.as_deref() == Some(session))
    }

    /// Opens a request for the app's `key`, or answers the one that key opened: its id, and
    /// whether it is new.
    pub fn open(&mut self, key: &str, title: &str, project: Option<&str>, now: u64) -> (String, bool) {
        if let Some(r) = self.saved.requests.iter().find(|r| r.key == key) {
            return (r.id.clone(), false);
        }
        let project = project.map(|p| cut(p, MAX_NAME)).filter(|p| !p.is_empty());
        let id = self.add(key.to_owned(), cut(title, MAX_TITLE), project, Kind::Asked, now);
        (id, true)
    }

    /// The request's Orchestrator session started as `session`. False for an unknown id.
    pub fn set_session(&mut self, id: &str, session: &str) -> bool {
        let Some(r) = self.saved.requests.iter_mut().find(|r| r.id == id) else { return false };
        r.session = Some(session.to_owned());
        r.held.clear();
        self.save();
        true
    }

    /// `line` waits for request `id`'s first session, which has not started.
    pub fn hold(&mut self, id: &str, line: &str) {
        if let Some(r) = self.saved.requests.iter_mut().find(|r| r.id == id && r.session.is_none()) {
            r.held.push(line.to_owned());
            self.save();
        }
    }

    /// Each open request still waiting to start, with the lines held for it.
    pub fn unstarted(&self) -> Vec<(String, Vec<String>)> {
        self.saved.requests.iter().filter(|r| r.closed.is_none() && r.session.is_none() && !r.held.is_empty()).map(|r| (r.id.clone(), r.held.clone())).collect()
    }

    /// Marks the request Done. False for an unknown id.
    pub fn close(&mut self, id: &str, by: ClosedBy, now: u64) -> bool {
        let Some(r) = self.saved.requests.iter_mut().find(|r| r.id == id) else { return false };
        r.closed = Some(Closed { by, at: now });
        self.save();
        true
    }

    /// Opens a Done request again. False for an unknown id.
    pub fn reopen(&mut self, id: &str) -> bool {
        let Some(r) = self.saved.requests.iter_mut().find(|r| r.id == id) else { return false };
        r.closed = None;
        self.save();
        true
    }

    /// A global `session` that no request tab holds sent `request_id` to `captain`. The first
    /// sender of an id keeps it, so a retry answers the first.
    pub fn record_send(&mut self, session: &str, request_id: &str, captain: CaptainRef) {
        let request = self.of_session(session).map(|r| r.id.clone());
        self.record(request, session, request_id, captain);
    }

    /// Request `request`'s tab, on `session` when it has started one, sent `request_id` to
    /// `captain`. The request remembers the Captain.
    pub fn record_request_send(&mut self, request: &str, session: Option<&str>, request_id: &str, captain: CaptainRef) {
        self.record(Some(request.to_owned()), session.unwrap_or_default(), request_id, captain);
    }

    fn record(&mut self, request: Option<String>, session: &str, request_id: &str, captain: CaptainRef) {
        let captain = captain.cleaned();
        if !self.saved.sent.iter().any(|s| s.id == request_id) {
            self.saved.sent.push(SentId { id: request_id.to_owned(), session: session.to_owned(), request: request.clone(), at: captain.at });
            let over = self.saved.sent.len().saturating_sub(SENT_KEEP);
            self.saved.sent.drain(..over);
        }
        if let Some(r) = request.and_then(|id| self.saved.requests.iter_mut().find(|r| r.id == id)).filter(|_| captain.path_ok()) {
            r.captains.retain(|c| !c.serves(&captain));
            r.captains.push(captain);
            let over = r.captains.len().saturating_sub(MAX_CAPTAINS);
            r.captains.drain(..over);
        }
        self.save();
    }

    /// Where `report`, from `captain` and answering `answers`, goes, leaving out what `delivered`
    /// already marked for it.
    pub fn route(&mut self, report: &str, answers: &[String], captain: &CaptainRef, now: u64) -> Vec<Route> {
        let mut to = self.routes(answers, captain, now);
        to.retain(|route| !self.was_delivered(report, route));
        to
    }

    /// Whether `report` was already typed into `route`.
    pub fn was_delivered(&self, report: &str, route: &Route) -> bool {
        self.saved.delivered.iter().any(|d| d.report == report && &d.to == route)
    }

    /// Where a report from `captain` answering `answers` goes, never empty:
    /// 1. Each answer that a global tab sent, matched exactly: the request whose tab sent it, else
    ///    that session's request, or the session itself when no request holds it.
    /// 2. Only when none matched: the newest open request that sent to this Captain, else its open
    ///    "Reports from Captain X" request, opened here when there is none.
    pub fn routes(&mut self, answers: &[String], captain: &CaptainRef, now: u64) -> Vec<Route> {
        let mut to: Vec<Route> = vec![];
        for sent in answers.iter().filter_map(|a| self.saved.sent.iter().find(|s| &s.id == a)) {
            let route = match (sent.request.as_deref().and_then(|id| self.get(id)), self.of_session(&sent.session)) {
                (Some(r), _) | (None, Some(r)) => Route::Request(r.id.clone()),
                (None, None) if sent.request.is_none() || !sent.session.is_empty() => Route::Session(sent.session.clone()),
                (None, None) => continue,
            };
            if !to.contains(&route) {
                to.push(route);
            }
        }
        if to.is_empty() {
            to.push(Route::Request(self.fallback(captain, now)));
        }
        to
    }

    /// The newest open request that sent to `captain`, else its open "Reports from Captain X"
    /// request, opened when there is none.
    fn fallback(&mut self, captain: &CaptainRef, now: u64) -> String {
        let captain = captain.clone().cleaned();
        // One per Captain, by machine and exact path, even for a path no Captain record keeps. A
        // path over `MAX_PATH` bytes is no part of the key: cutting it would break the match, so
        // one per machine holds them.
        let path = if captain.path.len() <= MAX_PATH { captain.path.as_str() } else { "" };
        let key = format!("captain-reports/{}/{path}", captain.machine);
        let open_for = |kind: Kind| {
            self.saved
                .requests
                .iter()
                .filter(|r| r.kind == kind && r.closed.is_none() && (r.captains.iter().any(|c| c.serves(&captain)) || (kind == Kind::CaptainReports && r.key == key)))
                .max_by_key(|r| r.opened_at)
                .map(|r| r.id.clone())
        };
        if let Some(id) = open_for(Kind::Asked).or_else(|| open_for(Kind::CaptainReports)) {
            return id;
        }
        let name = if captain.project.is_empty() { &captain.path } else { &captain.project };
        let title = cut(&format!("Reports from Captain {name}"), MAX_TITLE);
        let project = Some(captain.project.clone()).filter(|p| !p.is_empty());
        let id = self.add(key, title, project, Kind::CaptainReports, now);
        if let Some(r) = self.saved.requests.iter_mut().find(|r| r.id == id).filter(|_| captain.path_ok()) {
            r.captains.push(CaptainRef { at: now, ..captain });
        }
        self.save();
        id
    }

    /// `report` was typed into `to`.
    pub fn delivered(&mut self, report: &str, to: &Route) {
        self.saved.delivered.push(Delivered { report: report.to_owned(), to: to.clone() });
        let over = self.saved.delivered.len().saturating_sub(DELIVERED_KEEP);
        self.saved.delivered.drain(..over);
        self.save();
    }

    /// Once ever: the old Orchestrator conversation in `session`, if there was one, becomes the
    /// Done request "Earlier conversation", closed by the machine. Its id when this call made it.
    pub fn migrate(&mut self, session: Option<&str>, now: u64) -> Option<String> {
        if self.saved.migrated {
            return None;
        }
        self.saved.migrated = true;
        let made = session.filter(|s| self.of_session(s).is_none()).map(|session| {
            let id = self.add("earlier".to_owned(), EARLIER_TITLE.to_owned(), None, Kind::Earlier, now);
            if let Some(r) = self.saved.requests.iter_mut().find(|r| r.id == id) {
                r.session = Some(session.to_owned());
                r.closed = Some(Closed { by: ClosedBy::Orchestrator, at: now });
            }
            id
        });
        self.save();
        made
    }

    /// Adds a request with a new id, drops what the cap asks, and saves.
    fn add(&mut self, key: String, title: String, project: Option<String>, kind: Kind, now: u64) -> String {
        let id = loop {
            let id = new_id();
            if self.get(&id).is_none() {
                break id;
            }
        };
        self.saved.requests.push(UserRequest {
            id: id.clone(),
            key,
            title,
            project,
            opened_at: now,
            session: None,
            stands: Stands::default(),
            closed: None,
            captains: vec![],
            last: None,
            last_user_at: None,
            replies: 0,
            kind,
            held: vec![],
        });
        self.trim();
        self.save();
        id
    }

    /// Past `KEEP`, drops the Done requests closed longest ago, with the ids their session sent.
    /// Open requests stay, even past the cap.
    fn trim(&mut self) {
        while self.saved.requests.len() > KEEP {
            let oldest_done = self.saved.requests.iter().enumerate().filter_map(|(i, r)| r.closed.map(|c| (c.at, i))).min();
            let Some((_, i)) = oldest_done else { break };
            let gone = self.saved.requests.remove(i);
            self.saved.sent.retain(|s| s.request.as_ref() != Some(&gone.id) && (gone.session.as_ref() != Some(&s.session)));
            let route = Route::Request(gone.id);
            self.saved.delivered.retain(|d| d.to != route);
        }
    }

    /// Open requests, then Done ones from the last 14 days, newest first in each, at most 60.
    /// `tab_of` finds a session's tab.
    pub fn glances(&self, now: u64, tab_of: impl Fn(&str) -> Option<TabId>) -> Vec<RequestGlance> {
        let mut shown: Vec<&UserRequest> = self.saved.requests.iter().filter(|r| r.closed.is_none_or(|c| now.saturating_sub(c.at) <= GLANCE_DONE_MS)).collect();
        shown.sort_by_key(|r| (r.closed.is_some(), Reverse(r.opened_at)));
        shown.truncate(MAX_GLANCES);
        shown.into_iter().map(|r| r.glance(r.session.as_deref().and_then(&tab_of))).collect()
    }

    /// The 60 newest requests opened before `before`, or of all, newest first.
    pub fn page(&self, before: Option<u64>, tab_of: impl Fn(&UserRequest) -> Option<TabId>) -> Vec<RequestGlance> {
        let mut shown: Vec<&UserRequest> = self.saved.requests.iter().filter(|r| before.is_none_or(|b| r.opened_at < b)).collect();
        shown.sort_by_key(|r| Reverse(r.opened_at));
        shown.into_iter().take(MAX_GLANCES).map(|r| r.glance(tab_of(r))).collect()
    }

    /// The user typed into request `id`'s session: nothing waits on them any more, and a Done
    /// request opens again.
    pub fn user_typed(&mut self, id: &str, now: u64) -> bool {
        let Some(r) = self.saved.requests.iter_mut().find(|r| r.id == id) else { return false };
        (r.stands.needs, r.stands.needs_at, r.last_user_at, r.closed) = (String::new(), None, Some(now), None);
        self.save();
        true
    }

    /// Writes "Where this stands" for request `id`: each text cut to one line and capped, the
    /// title at 80 characters, every other text at 300, at most 6 links of 60. Setting `needs`
    /// stamps `needs_at`; clearing it clears that.
    pub fn stands_update(&mut self, id: &str, patch: StandsPatch, now: u64) -> Result<(), String> {
        let Some(r) = self.saved.requests.iter_mut().find(|r| r.id == id) else { return Err(format!("No request {id}.")) };
        if let Some(title) = patch.title.map(|t| cut(&t, MAX_TITLE)).filter(|t| !t.is_empty()) {
            r.title = title;
        }
        let stands = &mut r.stands;
        for (field, text) in [(&mut stands.asked, patch.asked), (&mut stands.done, patch.done), (&mut stands.left, patch.left)] {
            if let Some(text) = text {
                *field = cut(&text, MAX_WHAT);
            }
        }
        if let Some(needs) = patch.needs.map(|n| cut(&n, MAX_WHAT)) {
            if needs != stands.needs {
                stands.needs_at = (!needs.is_empty()).then_some(now);
                stands.needs = needs;
            }
        }
        if let Some(links) = patch.links {
            stands.links = links.iter().map(|l| cut(l, MAX_NAME)).filter(|l| !l.is_empty()).take(MAX_LINKS).collect();
        }
        stands.at = now;
        self.save();
        Ok(())
    }

    /// One line per open request other than `except`, newest first, at most 12: its id, title,
    /// project and what is done, left and needed, each cut to 120 characters.
    pub fn live_digest(&self, except: Option<&str>) -> Vec<String> {
        let mut open: Vec<&UserRequest> = self.saved.requests.iter().filter(|r| r.closed.is_none() && Some(r.id.as_str()) != except).collect();
        open.sort_by_key(|r| Reverse(r.opened_at));
        open.into_iter()
            .take(DIGEST_LINES)
            .map(|r| {
                let mut parts = vec![r.id.clone(), r.title.clone()];
                parts.extend(r.project.as_ref().map(|p| format!("#{p}")));
                for (name, text) in [("done", &r.stands.done), ("left", &r.stands.left), ("needs", &r.stands.needs)] {
                    if !text.is_empty() {
                        parts.push(format!("{name}: {}", text.chars().take(DIGEST_TEXT).collect::<String>()));
                    }
                }
                parts.join(" · ")
            })
            .collect()
    }

    /// Writes a sibling file only this user may read, flushes it to disk, and renames it over the
    /// last save, so a failed or cut write keeps that one.
    fn save(&self) {
        let Some(file) = &self.file else { return };
        let saved = (|| {
            use std::io::Write;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = file.with_extension("json.tmp");
            let _ = std::fs::remove_file(&tmp);
            let written = crate::private::file().create_new(true).open(&tmp).and_then(|mut f| {
                f.write_all(&serde_json::to_vec(&self.saved).expect("requests serialize"))?;
                f.sync_all()
            });
            if let Err(e) = written {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            std::fs::rename(&tmp, file)?;
            // The rename itself lasts once the folder is flushed too.
            #[cfg(unix)]
            if let Some(dir) = file.parent() {
                std::fs::File::open(dir)?.sync_all()?;
            }
            Ok(())
        })();
        if let Err(e) = saved {
            eprintln!("pinned: requests did not save: {e}");
        }
    }
}

/// The request a tab's environment names it the session of.
pub fn of_env(env: &[(String, String)]) -> Option<&str> {
    env.iter().find(|(k, _)| k == ENV).map(|(_, v)| v.as_str())
}

/// What the app asks to open: one paragraph of a new request.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct NewRequest {
    /// The app's idempotency key for this paragraph.
    pub key: String,
    pub title: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestAction {
    Done,
    Reopen,
    /// A resting tab on the request's session, for one whose tab closed.
    Open,
}

/// How a new request's first message went: its session started on it, or it waits for a free
/// place among the live request sessions.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Start {
    Started,
    Queued,
}

/// The answer to one `NewRequest`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub key: String,
    pub id: String,
    pub tab: TabId,
    pub handoff: Start,
}

/// One said thing of a request's thread.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub who: Who,
    pub text: String,
    pub at: u64,
    /// The Captain's project, on a report.
    #[serde(default)]
    pub captain: Option<String>,
}

/// A plan kept for a request after its Captain's tab went, named by that Captain.
#[derive(Serialize, Clone, Debug)]
pub struct RequestPlan {
    pub machine: MachineId,
    pub path: String,
    pub plan: crate::plan::PlanView,
}

/// The answer to `Call::UserRequest`.
#[derive(Serialize, Clone, Debug)]
pub struct UserRequestView {
    pub glance: RequestGlance,
    pub stands: Stands,
    pub items: Vec<Item>,
    pub plans: Vec<RequestPlan>,
}

/// The request a prefixed request id names: `q…/r-1` gives `q…`.
pub fn request_of(request_id: &str) -> Option<&str> {
    let (head, _) = request_id.split_once('/')?;
    let letters = head.strip_prefix('q')?;
    (letters.len() == ID_LEN && letters.bytes().all(|b| ID_LETTERS.contains(&b))).then_some(head)
}

/// `q` and 10 letters drawn from the standard library's randomly seeded hasher.
fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let bits = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let letters: String = (0..ID_LEN).map(|n| ID_LETTERS[((bits >> (5 * n)) & 31) as usize] as char).collect();
    format!("q{letters}")
}

/// `text` as one line of at most `max` characters, by plan.rs's rules for a name: line breaks and
/// control characters become spaces, direction marks and invisible characters go, and runs of
/// spaces become one.
fn cut(text: &str, max: usize) -> String {
    use crate::plan::{direction_mark, invisible, line_separator};
    let flat: String = text.chars().filter(|c| !direction_mark(*c) && !invisible(*c)).map(|c| if c.is_control() || line_separator(c) { ' ' } else { c }).collect();
    let one = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(max).collect::<String>().trim_end().to_owned()
}

impl CaptainRef {
    /// Each name one line of at most 60 characters. The path stays byte for byte, since it is
    /// the Captain tab's exact folder, which the app matches.
    fn cleaned(self) -> Self {
        CaptainRef { machine: cut(&self.machine, MAX_NAME), project: cut(&self.project, MAX_NAME), machine_name: cut(&self.machine_name, MAX_NAME), ..self }
    }

    /// A path with a control or line-separator character names no folder a tab runs in, and one
    /// over `MAX_PATH` bytes is not kept: cutting it would break the exact match.
    fn path_ok(&self) -> bool {
        self.path.len() <= MAX_PATH && !self.path.chars().any(|c| c.is_control() || crate::plan::line_separator(c))
    }

    /// Whether `report`'s Captain is this one: the same machine and folder, or for a box, the same
    /// box. A box's folder name says nothing: every clone of a repo named `site` is `site`.
    fn serves(&self, report: &CaptainRef) -> bool {
        if self.boxed {
            !self.machine.is_empty() && self.machine == report.machine
        } else {
            self.machine == report.machine && self.path == report.path
        }
    }
}

impl UserRequest {
    pub fn glance(&self, tab: Option<TabId>) -> RequestGlance {
        RequestGlance {
            id: self.id.clone(),
            title: self.title.clone(),
            project: self.project.clone(),
            opened_at: self.opened_at,
            kind: self.kind,
            closed_at: self.closed.map(|c| c.at),
            needs: self.stands.needs.clone(),
            needs_at: self.stands.needs_at,
            last_user_at: self.last_user_at,
            last_who: self.last.as_ref().map(|l| l.who),
            last_text: self.last.as_ref().map(|l| l.text.clone()).unwrap_or_default(),
            last_at: self.last.as_ref().map(|l| l.at),
            replies: self.replies,
            captains: self.captains.iter().map(|c| GlanceCaptain { machine: c.machine.clone(), path: c.path.clone(), project: c.project.clone(), machine_name: c.machine_name.clone() }).collect(),
            tab,
            asked: self.stands.asked.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-user-requests-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn captain(machine: &str, path: &str, project: &str, at: u64) -> CaptainRef {
        CaptainRef { machine: machine.into(), path: path.into(), project: project.into(), machine_name: "MacBook".into(), boxed: false, at }
    }

    /// Opens a request whose session is `s-<key>`.
    fn started(store: &mut UserRequests, key: &str, now: u64) -> String {
        let (id, _) = store.open(key, &format!("Ask {key}"), Some("site"), now);
        assert!(store.set_session(&id, &format!("s-{key}")));
        id
    }

    fn to(id: &str) -> Route {
        Route::Request(id.to_owned())
    }

    #[test]
    fn a_retry_with_the_same_key_opens_one_request() {
        let dir = temp("retry");
        let mut store = UserRequests::load(Some(&dir));
        let (first, opened) = store.open("k-1", "Fix the login", Some("site"), 1_000);
        assert!(opened);
        assert_eq!(store.open("k-1", "Fix the login again", Some("blog"), 2_000), (first.clone(), false));
        let mut again = UserRequests::load(Some(&dir));
        assert_eq!(again.open("k-1", "Fix the login", Some("site"), 3_000), (first.clone(), false));
        assert_eq!(again.all().len(), 1);
        let r = again.get(&first).unwrap();
        assert_eq!((r.title.as_str(), r.project.as_deref(), r.opened_at, r.kind), ("Fix the login", Some("site"), 1_000, Kind::Asked));
        assert_eq!((first.len(), first.starts_with('q')), (11, true));
        assert_eq!(request_of(&format!("{first}/r-1")), Some(first.as_str()));
        assert_eq!(request_of("r-1"), None);
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(dir.join(FILE)).unwrap().permissions()) & 0o777, 0o600);
        assert_ne!(again.open("k-2", "Another", None, 4_000).0, first);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unmatched_report_goes_to_the_newest_open_request_for_that_captain_else_one_per_captain() {
        let mut store = UserRequests::load(None);
        let site = captain("m1", "/p/site", "site", 0);
        let a = started(&mut store, "a", 1_000);
        let b = started(&mut store, "b", 2_000);
        let c = started(&mut store, "c", 3_000);
        store.record_send("s-a", &format!("{a}/r-1"), captain("m1", "/p/site", "site", 1_100));
        store.record_send("s-b", &format!("{b}/r-1"), captain("m1", "/p/site", "site", 2_100));
        store.record_send("s-c", &format!("{c}/r-1"), captain("m1", "/p/blog", "blog", 3_100));

        assert_eq!(store.route("rep-1", &[], &site, 4_000), vec![to(&b)]);
        assert_eq!(store.route("rep-2", &["old-id".to_owned()], &site, 4_000), vec![to(&b)]);
        store.close(&b, ClosedBy::User, 4_500);
        assert_eq!(store.route("rep-3", &[], &site, 5_000), vec![to(&a)]);
        store.close(&a, ClosedBy::User, 5_500);

        let reports = store.route("rep-4", &[], &site, 6_000);
        let [Route::Request(x)] = reports.as_slice() else { panic!("one request, not {reports:?}") };
        let r = store.get(x).unwrap();
        assert_eq!((r.title.as_str(), r.kind, r.project.as_deref(), r.opened_at), ("Reports from Captain site", Kind::CaptainReports, Some("site"), 6_000));
        assert_eq!(store.route("rep-5", &[], &site, 7_000), vec![to(x)]);
        let api = store.route("rep-6", &[], &captain("m2", "/p/api", "api", 0), 8_000);
        assert_eq!(api.len(), 1);
        assert_ne!(api[0], to(x));
        assert_eq!(store.all().iter().filter(|r| r.kind == Kind::CaptainReports).count(), 2);
        assert_eq!(store.route("rep-7", &[], &captain("m1", "/p/blog", "blog", 0), 9_000), vec![to(&c)]);
    }

    #[test]
    fn a_report_naming_two_requests_routes_to_both() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        let b = started(&mut store, "b", 2_000);
        store.record_send("s-a", &format!("{a}/r-1"), captain("m1", "/p/site", "site", 1_100));
        store.record_send("s-b", &format!("{b}/r-1"), captain("m1", "/p/site", "site", 2_100));
        let answers = [format!("{a}/r-1"), format!("{b}/r-1")];
        assert_eq!(store.route("rep-1", &answers, &captain("m1", "/p/site", "site", 0), 3_000), vec![to(&a), to(&b)]);
    }

    #[test]
    fn the_old_orchestrator_becomes_one_earlier_request_once() {
        let dir = temp("earlier");
        let mut store = UserRequests::load(Some(&dir));
        let earlier = store.migrate(Some("s-old"), 5_000).expect("made");
        assert_eq!(store.migrate(Some("s-other"), 6_000), None);
        assert_eq!(UserRequests::load(Some(&dir)).migrate(Some("s-old"), 7_000), None);
        let store = UserRequests::load(Some(&dir));
        assert_eq!(store.all().len(), 1);
        let r = store.get(&earlier).unwrap();
        assert_eq!(
            (r.title.as_str(), r.kind, r.session.as_deref(), r.closed, r.opened_at),
            ("Earlier conversation", Kind::Earlier, Some("s-old"), Some(Closed { by: ClosedBy::Orchestrator, at: 5_000 }), 5_000)
        );
        assert_eq!(store.of_session("s-old").map(|r| r.id.as_str()), Some(earlier.as_str()));
        let mut none = UserRequests::load(None);
        assert_eq!(none.migrate(None, 1), None);
        assert_eq!(none.migrate(Some("s-late"), 2), None);
        assert!(none.all().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn open_requests_survive_the_500_cap() {
        let mut store = UserRequests::load(None);
        let open: Vec<String> = (0..10).map(|n| store.open(&format!("open-{n}"), "Still open", None, n).0).collect();
        let done: Vec<String> = (0..490)
            .map(|n| {
                let (id, _) = store.open(&format!("done-{n}"), "Done", None, 100 + n);
                store.close(&id, ClosedBy::User, 1_000 + n);
                id
            })
            .collect();
        assert_eq!(store.all().len(), 500);
        let more: Vec<String> = (0..6).map(|n| store.open(&format!("more-{n}"), "More", None, 2_000 + n).0).collect();
        assert_eq!(store.all().len(), 500);
        assert!(more.iter().all(|id| store.get(id).is_some()));
        assert!(open.iter().all(|id| store.get(id).is_some()), "every open request is kept");
        assert_eq!(done.iter().filter(|id| store.get(id).is_none()).count(), 6);
        assert!(done[..6].iter().all(|id| store.get(id).is_none()), "the oldest Done ones go first");
        assert!(store.get(&done[6]).is_some());
    }

    #[test]
    fn a_store_that_does_not_parse_is_kept_aside() {
        let dir = temp("corrupt");
        let corrupt = br#"{"requests":[{"id":"qaaaaaaaaaa","key":"k-1","tit"#;
        std::fs::write(dir.join(FILE), corrupt).unwrap();
        let mut store = UserRequests::load(Some(&dir));
        assert!(store.all().is_empty());
        assert_eq!(std::fs::read(dir.join(format!("{FILE}.bad"))).unwrap(), corrupt, "the file is kept aside as it was");
        assert!(!dir.join(FILE).exists());
        let (id, _) = store.open("k-2", "After the bad file", None, 1_000);
        assert_eq!(UserRequests::load(Some(&dir)).get(&id).map(|r| r.title.as_str()), Some("After the bad file"));
        assert_eq!(std::fs::read(dir.join(format!("{FILE}.bad"))).unwrap(), corrupt, "the write left it alone");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stands_text_is_one_line_and_capped() {
        let mut store = UserRequests::load(None);
        let (id, _) = store.open("k-1", "Ship\nthe\u{2028}form", None, 1_000);
        assert_eq!(store.get(&id).unwrap().title, "Ship the form");
        let patch = StandsPatch {
            title: Some(format!("New\ttitle {}", "t".repeat(100))),
            done: Some("Built the form\nand the API\u{2028}then \u{202E}shipped\u{200B} it".into()),
            left: Some("l".repeat(400)),
            needs: Some("Pick a colour\r\n".into()),
            links: Some((1..=8).map(|n| format!("https://a.example/{n}\n{}", "x".repeat(60))).collect()),
            ..Default::default()
        };
        store.stands_update(&id, patch, 2_000).unwrap();
        let r = store.get(&id).unwrap();
        assert_eq!(r.title, format!("New title {}", "t".repeat(70)));
        assert_eq!(r.stands.done, "Built the form and the API then shipped it");
        assert_eq!(r.stands.left, "l".repeat(300));
        assert_eq!((r.stands.needs.as_str(), r.stands.needs_at, r.stands.at), ("Pick a colour", Some(2_000), 2_000));
        assert_eq!(r.stands.links.len(), 6);
        assert_eq!(r.stands.links[0], format!("https://a.example/1 {}", "x".repeat(40)));
        assert_eq!(r.stands.asked, "");

        store.stands_update(&id, StandsPatch { needs: Some(" ".into()), ..Default::default() }, 3_000).unwrap();
        let r = store.get(&id).unwrap();
        assert_eq!((r.stands.needs.as_str(), r.stands.needs_at, r.stands.done.as_str()), ("", None, "Built the form and the API then shipped it"));
        assert_eq!(store.stands_update("qnope", StandsPatch::default(), 4_000), Err("No request qnope.".to_owned()));
    }

    #[test]
    fn the_digest_lists_other_open_requests_newest_first_at_most_12() {
        let mut store = UserRequests::load(None);
        let ids: Vec<String> = (1..=15).map(|n| store.open(&format!("k-{n}"), &format!("Task {n}"), Some("site"), n * 1_000).0).collect();
        store.close(&ids[13], ClosedBy::User, 20_000);
        let stands = StandsPatch { done: Some("Wrote it".into()), left: Some("Ship it".into()), needs: Some("Pick a colour".into()), ..Default::default() };
        store.stands_update(&ids[12], stands, 21_000).unwrap();
        store.stands_update(&ids[11], StandsPatch { done: Some("d".repeat(200)), ..Default::default() }, 21_000).unwrap();

        let digest = store.live_digest(Some(&ids[14]));
        assert_eq!(digest.len(), 12);
        assert_eq!(digest[0], format!("{} · Task 13 · #site · done: Wrote it · left: Ship it · needs: Pick a colour", ids[12]));
        assert_eq!(digest[1], format!("{} · Task 12 · #site · done: {}", ids[11], "d".repeat(120)));
        assert_eq!(digest[2], format!("{} · Task 11 · #site", ids[10]));
        assert_eq!(digest[11], format!("{} · Task 2 · #site", ids[1]));
        assert_eq!(store.live_digest(None)[0], format!("{} · Task 15 · #site", ids[14]));
    }

    #[test]
    fn a_forged_or_old_id_routes_nowhere_but_the_fallback() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        let b = started(&mut store, "b", 2_000);
        store.record_send("s-a", &format!("{a}/r-1"), captain("m1", "/p/site", "site", 1_100));
        store.record_send("s-b", &format!("{b}/r-1"), captain("m1", "/p/site", "site", 2_100));
        let forged = [format!("{a}/r-forged"), format!("{a}/r-1 ")];
        assert_eq!(store.route("rep-1", &forged, &captain("m1", "/p/site", "site", 0), 3_000), vec![to(&b)]);
    }

    #[test]
    fn a_report_answering_an_anchor_id_goes_to_the_anchor() {
        let mut store = UserRequests::load(None);
        store.record_send("s-anchor", "r-1", captain("m1", "/p/site", "site", 500));
        let a = started(&mut store, "a", 1_000);
        store.record_send("s-a", &format!("{a}/r-1"), captain("m1", "/p/site", "site", 1_100));
        let site = captain("m1", "/p/site", "site", 0);
        assert_eq!(store.route("rep-1", &["r-1".to_owned()], &site, 2_000), vec![Route::Session("s-anchor".into())]);
        store.record_send("s-a", "r-1", captain("m1", "/p/site", "site", 2_100));
        assert_eq!(store.route("rep-2", &["r-1".to_owned()], &site, 3_000), vec![Route::Session("s-anchor".into())], "a retry answers the first sender");
    }

    #[test]
    fn a_retry_after_the_first_of_two_deliveries_reaches_the_second() {
        let dir = temp("delivered");
        let mut store = UserRequests::load(Some(&dir));
        let a = started(&mut store, "a", 1_000);
        let b = started(&mut store, "b", 2_000);
        store.record_send("s-a", &format!("{a}/r-1"), captain("m1", "/p/site", "site", 1_100));
        store.record_send("s-b", &format!("{b}/r-1"), captain("m1", "/p/site", "site", 2_100));
        let answers = [format!("{a}/r-1"), format!("{b}/r-1")];
        let site = captain("m1", "/p/site", "site", 0);
        assert_eq!(store.route("rep-1", &answers, &site, 3_000), vec![to(&a), to(&b)]);
        store.delivered("rep-1", &to(&a));
        let mut store = UserRequests::load(Some(&dir));
        assert_eq!(store.route("rep-1", &answers, &site, 4_000), vec![to(&b)]);
        store.delivered("rep-1", &to(&b));
        assert_eq!(store.route("rep-1", &answers, &site, 5_000), vec![]);
        assert_eq!(store.route("rep-2", &answers, &site, 6_000), vec![to(&a), to(&b)]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_report_after_clear_reaches_the_request_that_sent_it() {
        let dir = temp("clear");
        let mut store = UserRequests::load(Some(&dir));
        store.record_send("s-anchor", "r-9", captain("m1", "/p/site", "site", 500));
        let a = started(&mut store, "a", 1_000);
        store.record_request_send(&a, Some("s-a"), &format!("{a}/r-1"), captain("m1", "/p/site", "site", 1_100));
        assert!(store.set_session(&a, "s-a2"), "the user typed /clear in A");
        let site = captain("m1", "/p/site", "site", 0);
        assert_eq!(store.route("rep-1", &[format!("{a}/r-1")], &site, 2_000), vec![to(&a)]);
        let mut store = UserRequests::load(Some(&dir));
        assert_eq!(store.route("rep-2", &[format!("{a}/r-1")], &site, 3_000), vec![to(&a)], "the request id survives a restart");
        assert_eq!(store.route("rep-3", &["r-9".to_owned()], &site, 4_000), vec![Route::Session("s-anchor".into())], "a session no request holds keeps its route");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_sent_id_saved_before_requests_were_kept_still_routes_by_session() {
        let dir = temp("old-sent");
        std::fs::write(dir.join(FILE), r#"{"requests":[],"sent":[{"id":"r-1","session":"s-anchor","at":5}]}"#).unwrap();
        let mut store = UserRequests::load(Some(&dir));
        assert_eq!(store.route("rep-1", &["r-1".to_owned()], &captain("m1", "/p/site", "site", 0), 1_000), vec![Route::Session("s-anchor".into())]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_request_tab_without_a_session_yet_still_gets_its_report() {
        let mut store = UserRequests::load(None);
        let (a, _) = store.open("a", "Ask a", Some("site"), 1_000);
        store.record_request_send(&a, None, &format!("{a}/r-1"), captain("m1", "/p/site", "site", 1_100));
        assert_eq!(store.get(&a).unwrap().captains.len(), 1, "the request remembers its Captain");
        assert_eq!(store.route("rep-1", &[format!("{a}/r-1")], &captain("m1", "/p/site", "site", 0), 2_000), vec![to(&a)]);
    }

    #[test]
    fn two_unmatched_reports_from_one_captain_share_one_thread() {
        let mut store = UserRequests::load(None);
        let refused = captain("m1", "/p/\nsite", "site", 0);
        let first = store.route("rep-1", &[], &refused, 1_000);
        assert_eq!(store.route("rep-2", &[], &refused, 2_000), first, "a path the store refuses still finds its reports thread");
        let elsewhere = store.route("rep-3", &[], &captain("m1", "/q/\nsite", "site", 0), 3_000);
        assert_ne!(elsewhere, first, "never matched by folder name");
        let other_machine = store.route("rep-4", &[], &captain("m2", "/p/\nsite", "site", 0), 4_000);
        assert_ne!(other_machine, first);
        assert_eq!(store.all().iter().filter(|r| r.kind == Kind::CaptainReports).count(), 3);
    }

    #[test]
    fn a_box_captain_matches_by_machine() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        let b = started(&mut store, "b", 2_000);
        let boxed = CaptainRef { machine: "box-7".into(), path: String::new(), project: "site".into(), machine_name: "box".into(), boxed: true, at: 1_100 };
        store.record_send("s-a", &format!("{a}/r-1"), boxed);
        store.record_send("s-b", &format!("{b}/r-1"), captain("m1", "/p/site", "site", 2_100));
        let from_box = captain("box-7", "/home/boxuser/work/site", "site", 0);
        assert_eq!(store.route("rep-1", &[], &from_box, 3_000), vec![to(&a)]);
        let from_next_box = captain("box-9", "/home/boxuser/work/site", "site", 0);
        let routes = store.route("rep-2", &[], &from_next_box, 4_000);
        assert!(!routes.contains(&to(&a)), "another box with the same folder name is another Captain: {routes:?}");
        assert_eq!(store.route("rep-3", &[], &captain("m1", "/p/site", "site", 0), 5_000), vec![to(&b)]);
    }

    #[test]
    fn captain_fields_with_newlines_are_flattened_and_capped() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        let odd = CaptainRef { machine: "m\r1".into(), path: format!("/p/{}", "a".repeat(400)), project: "si\u{2028}te".into(), machine_name: "Mac\nmini".into(), boxed: false, at: 1_100 };
        store.record_send("s-a", &format!("{a}/r-1"), odd.clone());
        store.record_send("s-a", &format!("{a}/r-2"), CaptainRef { path: "/p/\nsite".into(), ..odd.clone() });
        let c = &store.get(&a).unwrap().captains;
        assert_eq!(c.len(), 1, "a path with a line break names no tab's folder");
        assert_eq!((c[0].machine.as_str(), c[0].project.as_str(), c[0].machine_name.as_str()), ("m 1", "si te", "Mac mini"));
        assert_eq!(c[0].path, format!("/p/{}", "a".repeat(400)));
        let glance = &store.glances(2_000, |_| None)[0];
        assert_eq!(glance.captains, vec![GlanceCaptain { machine: "m 1".into(), path: format!("/p/{}", "a".repeat(400)), project: "si te".into(), machine_name: "Mac mini".into() }]);

        store.close(&a, ClosedBy::User, 1_500);
        let routes = store.route("rep-1", &[], &odd, 2_000);
        let [Route::Request(x)] = routes.as_slice() else { panic!("one request, not {routes:?}") };
        assert_eq!(store.get(x).unwrap().title, "Reports from Captain si te");

        let b = started(&mut store, "b", 3_000);
        for n in 0..10 {
            store.record_send("s-b", &format!("{b}/r-{n}"), captain(&format!("m{n}"), "/p/site", "site", 3_000 + n));
        }
        let kept: Vec<&str> = store.get(&b).unwrap().captains.iter().map(|c| c.machine.as_str()).collect();
        assert_eq!(kept, ["m2", "m3", "m4", "m5", "m6", "m7", "m8", "m9"]);
    }

    #[test]
    fn a_captain_path_is_kept_exactly_as_the_tabs_cwd() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        let cwd = format!("/Users/ana/My  Projects/{}", "x".repeat(296));
        assert_eq!(cwd.chars().count(), 320);
        store.record_send("s-a", &format!("{a}/r-1"), captain("m1", &cwd, "site", 1_100));
        assert_eq!(store.glances(2_000, |_| None)[0].captains[0].path, cwd);
        assert_eq!(store.route("rep-1", &[], &captain("m1", &cwd, "site", 0), 3_000), vec![to(&a)], "a report from that tab's folder finds the request");
        store.close(&a, ClosedBy::User, 3_500);
        let routes = store.route("rep-2", &[], &captain("m1", &cwd, "site", 0), 4_000);
        let [Route::Request(x)] = routes.as_slice() else { panic!("one request, not {routes:?}") };
        assert_eq!(store.get(x).unwrap().captains[0].path, cwd, "the reports request keeps the folder exact too");
    }

    #[test]
    fn glances_hold_open_requests_and_done_ones_from_the_last_14_days() {
        let mut store = UserRequests::load(None);
        let day = 24 * 60 * 60 * 1000;
        let old = started(&mut store, "old", 1_000);
        store.close(&old, ClosedBy::User, 2_000);
        let recent = started(&mut store, "recent", 3_000);
        store.close(&recent, ClosedBy::Orchestrator, 15 * day);
        let open = started(&mut store, "open", 4_000);
        store.stands_update(&open, StandsPatch { needs: Some("Say yes".into()), ..Default::default() }, 5_000).unwrap();
        let now = 15 * day + 1_000;
        let glances = store.glances(now, |session| (session == "s-open").then(|| "tab-1".to_owned()));
        let ids: Vec<&str> = glances.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, [open.as_str(), recent.as_str()]);
        assert_eq!(
            glances[0],
            RequestGlance {
                id: open.clone(),
                title: "Ask open".into(),
                project: Some("site".into()),
                opened_at: 4_000,
                kind: Kind::Asked,
                closed_at: None,
                needs: "Say yes".into(),
                needs_at: Some(5_000),
                last_user_at: None,
                last_who: None,
                last_text: String::new(),
                last_at: None,
                replies: 0,
                captains: vec![],
                tab: Some("tab-1".into()),
                asked: String::new(),
            }
        );
        assert_eq!((glances[1].closed_at, glances[1].tab.as_deref()), (Some(15 * day), None));
        for n in 0..70 {
            store.open(&format!("many-{n}"), "Many", None, 10_000 + n);
        }
        let many = store.glances(now, |_| None);
        assert_eq!(many.len(), 60);
        assert!(many.iter().all(|g| g.closed_at.is_none()), "open ones come first");
        assert_eq!(many[0].opened_at, 10_069);
    }

    /// A report from box `machine`, whose folder is `site` like every clone of a repo named site.
    fn box_report(machine: &str) -> CaptainRef {
        CaptainRef { machine: machine.into(), path: "/home/box/pinned/site".into(), project: "site".into(), machine_name: "box".into(), boxed: false, at: 0 }
    }

    #[test]
    fn two_boxes_with_the_same_folder_name_but_different_repos_do_not_share_a_request() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        let b = started(&mut store, "b", 2_000);
        let one = serde_json::json!({ "outcome": "iniciado", "machine": "box-1", "machineName": "box", "box": "row-1" });
        let two = serde_json::json!({ "outcome": "iniciado", "machine": "box-2", "machineName": "box", "box": "row-2" });
        store.record_send("s-a", &format!("{a}/r-1"), crate::machine::box_captain_ref(&one, "github.com/acme/site", 1_100));
        store.record_send("s-b", &format!("{b}/r-1"), crate::machine::box_captain_ref(&two, "github.com/other/site", 2_100));
        assert_eq!(store.route("rep-1", &[], &box_report("box-1"), 3_000), vec![to(&a)], "box 1's report goes to the request that sent to box 1");
        let three = store.route("rep-2", &[], &box_report("box-3"), 4_000);
        assert!(three != vec![to(&a)] && three != vec![to(&b)], "a third box named site shares neither: {three:?}");
        let blind = serde_json::json!({ "outcome": "iniciado" });
        let c = started(&mut store, "c", 5_000);
        store.record_send("s-c", &format!("{c}/r-1"), crate::machine::box_captain_ref(&blind, "github.com/acme/site", 5_100));
        assert_eq!(store.route("rep-3", &[], &box_report("box-1"), 6_000), vec![to(&a)], "a box with no id matches nothing in the fallback");
        assert_eq!(store.route("rep-4", &[format!("{c}/r-1")], &box_report("box-9"), 6_100), vec![to(&c)], "its exact id still routes it");
    }

    #[test]
    fn two_remote_captains_are_recorded_apart_with_machine_and_path() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        store.record_send("s-a", &format!("{a}/r-1"), crate::machine::remote_captain_ref("m-mini", "/w/site", 1_100));
        store.record_send("s-a", &format!("{a}/r-2"), crate::machine::remote_captain_ref("m-book", "/u/site", 1_200));
        let recorded: Vec<(String, String)> = store.get(&a).unwrap().captains.iter().map(|c| (c.machine.clone(), c.path.clone())).collect();
        assert_eq!(recorded, vec![("m-mini".to_owned(), "/w/site".to_owned()), ("m-book".to_owned(), "/u/site".to_owned())]);
        assert_eq!(store.route("rep-1", &[], &captain("m-book", "/u/site", "site", 0), 2_000), vec![to(&a)], "the fallback finds a remote Captain");
    }

    #[test]
    fn a_path_over_4096_bytes_is_not_recorded() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        let long = format!("/{}", "é".repeat(2_048));
        assert!(long.chars().count() <= 4096 && long.len() > 4096);
        store.record_send("s-a", &format!("{a}/r-1"), captain("m1", &long, "site", 1_100));
        let edge = format!("/{}", "p".repeat(4_095));
        store.record_send("s-a", &format!("{a}/r-2"), captain("m1", &edge, "site", 1_200));
        let paths: Vec<usize> = store.get(&a).unwrap().captains.iter().map(|c| c.path.len()).collect();
        assert_eq!(paths, vec![4_096], "only the path of 4096 bytes is kept, byte for byte");
    }

    #[test]
    fn a_report_path_over_4096_bytes_reaches_no_fallback_key() {
        let mut store = UserRequests::load(None);
        let long = format!("/{}", "é".repeat(2_048));
        let routes = store.route("rep-1", &[], &captain("m1", &long, "site", 0), 1_000);
        let [Route::Request(id)] = routes.as_slice() else { panic!("one request, not {routes:?}") };
        let r = store.get(id).unwrap();
        assert_eq!((r.key.as_str(), r.captains.len()), ("captain-reports/m1/", 0));
        let routes = store.route("rep-2", &[], &captain("m1", &format!("{long}x"), "site", 0), 2_000);
        assert_eq!(routes, vec![to(id)], "another path no record keeps joins it");
    }

    #[test]
    fn a_user_message_in_a_done_request_reopens_it() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        assert!(store.close(&a, ClosedBy::Orchestrator, 2_000));
        assert!(store.user_typed(&a, 3_000));
        let r = store.get(&a).unwrap();
        assert_eq!((r.closed.is_none(), r.last_user_at), (true, Some(3_000)));
    }
}
