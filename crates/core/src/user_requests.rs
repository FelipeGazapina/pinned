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
pub const MAX_LINKS: usize = 6;
pub const MAX_LAST: usize = 120;
pub const MAX_GLANCES: usize = 60;
/// How long a Done request stays in the glances.
pub const GLANCE_DONE_MS: u64 = 14 * 24 * 60 * 60 * 1000;
pub const DIGEST_LINES: usize = 12;
pub const DIGEST_TEXT: usize = 120;
pub const EARLIER_TITLE: &str = "Earlier conversation";
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
/// before migration.
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
    session: String,
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
        todo!("{key} {title} {project:?} {now}")
    }

    /// The request's Orchestrator session started as `session`. False for an unknown id.
    pub fn set_session(&mut self, id: &str, session: &str) -> bool {
        let Some(r) = self.saved.requests.iter_mut().find(|r| r.id == id) else { return false };
        r.session = Some(session.to_owned());
        self.save();
        true
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

    /// A global `session` sent `request_id` to `captain`. The first session to send an id keeps
    /// it, so a retry answers the first; the session's request remembers the Captain.
    pub fn record_send(&mut self, session: &str, request_id: &str, captain: CaptainRef) {
        todo!("{session} {request_id} {captain:?}")
    }

    /// Where `report`, from `captain` and answering `answers`, goes, leaving out what `delivered`
    /// already marked for it.
    /// 1. Each answer that a global session sent, matched exactly: that session's request, or the
    ///    session itself when no request holds it.
    /// 2. Only when none matched: the newest open request that sent to this Captain, else its open
    ///    "Reports from Captain X" request, opened here when there is none.
    pub fn route(&mut self, report: &str, answers: &[String], captain: &CaptainRef, now: u64) -> Vec<Route> {
        todo!("{report} {answers:?} {captain:?} {now}")
    }

    /// `report` was typed into `to`.
    pub fn delivered(&mut self, report: &str, to: &Route) {
        todo!("{report} {to:?}")
    }

    /// Once ever: the old Orchestrator conversation in `session`, if there was one, becomes the
    /// Done request "Earlier conversation". Its id when this call made it.
    pub fn migrate(&mut self, session: Option<&str>, now: u64) -> Option<String> {
        todo!("{session:?} {now}")
    }

    /// Open requests, then Done ones from the last 14 days, newest first in each, at most 60.
    /// `tab_of` finds a session's tab.
    pub fn glances(&self, now: u64, tab_of: impl Fn(&str) -> Option<TabId>) -> Vec<RequestGlance> {
        let _ = tab_of;
        todo!("{now}")
    }

    /// Writes "Where this stands" for request `id`: each text cut to one line and capped, the
    /// title at 80 characters, every other text at 300, at most 6 links of 60. Setting `needs`
    /// stamps `needs_at`; clearing it clears that.
    pub fn stands_update(&mut self, id: &str, patch: StandsPatch, now: u64) -> Result<(), String> {
        todo!("{id} {patch:?} {now}")
    }

    /// One line per open request other than `except`, newest first, at most 12: its id, title,
    /// project and what is done, left and needed, each cut to 120 characters.
    pub fn live_digest(&self, except: Option<&str>) -> Vec<String> {
        todo!("{except:?}")
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

/// The request a prefixed request id names: `q…/r-1` gives `q…`.
pub fn request_of(request_id: &str) -> Option<&str> {
    todo!("{request_id}")
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
        let done: Vec<String> = (0..495)
            .map(|n| {
                let (id, _) = store.open(&format!("done-{n}"), "Done", None, 100 + n);
                store.close(&id, ClosedBy::User, 1_000 + n);
                id
            })
            .collect();
        assert_eq!(store.all().len(), 505);
        store.open("one-more", "One more", None, 2_000);
        assert_eq!(store.all().len(), 500);
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
        assert_eq!(r.stands.links[0], format!("https://a.example/1 {}", "x".repeat(39)));
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
        assert_eq!(store.route("rep-2", &[], &from_next_box, 4_000), vec![to(&a)], "or by repo");
        assert_eq!(store.route("rep-3", &[], &captain("m1", "/p/site", "site", 0), 5_000), vec![to(&b)]);
    }

    #[test]
    fn captain_fields_with_newlines_are_flattened_and_capped() {
        let mut store = UserRequests::load(None);
        let a = started(&mut store, "a", 1_000);
        let odd = CaptainRef { machine: "m\r1".into(), path: format!("/p/\n{}", "a".repeat(400)), project: "si\u{2028}te".into(), machine_name: "Mac\nmini".into(), boxed: false, at: 1_100 };
        store.record_send("s-a", &format!("{a}/r-1"), odd.clone());
        let c = &store.get(&a).unwrap().captains[0];
        assert_eq!((c.machine.as_str(), c.project.as_str(), c.machine_name.as_str()), ("m 1", "si te", "Mac mini"));
        assert_eq!(c.path, format!("/p/ {}", "a".repeat(296)));
        let glance = &store.glances(2_000, |_| None)[0];
        assert_eq!(glance.captains, vec![GlanceCaptain { machine: "m 1".into(), path: format!("/p/ {}", "a".repeat(296)), project: "si te".into(), machine_name: "Mac mini".into() }]);

        store.close(&a, ClosedBy::User, 1_500);
        let [Route::Request(x)] = store.route("rep-1", &[], &odd, 2_000)[..] else { panic!("one request") };
        assert_eq!(store.get(&x).unwrap().title, "Reports from Captain si te");

        let b = started(&mut store, "b", 3_000);
        for n in 0..10 {
            store.record_send("s-b", &format!("{b}/r-{n}"), captain(&format!("m{n}"), "/p/site", "site", 3_000 + n));
        }
        let kept: Vec<&str> = store.get(&b).unwrap().captains.iter().map(|c| c.machine.as_str()).collect();
        assert_eq!(kept, ["m2", "m3", "m4", "m5", "m6", "m7", "m8", "m9"]);
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
}
