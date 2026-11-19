//! The hub's file list of each project this machine syncs, kept here at a revision and brought up
//! to date with what changed since, so a sync cycle in which nothing moved asks the hub nothing.
//!
//! `Feed` wraps the link and is the `sync::Hub` a cycle passes to `sync::sync`. Its `list` answers
//! from the project's mirror and asks the hub only for what changed since the mirror's revision,
//! and only when something says the hub moved: a `SyncChanged` notice, this machine's own write, a
//! new connection, or `RECHECK`. Against a hub from before revisions it lists once per project per
//! cycle, as the loop did before.
//!
//! A mirror is the hub's list, so there is one per project key. What a checkout agreed with the
//! hub stays per checkout in `sync::State`: two checkouts of one project share a mirror and never
//! share agreement.

use crate::sync::{self, Changes, Entry, File, ModeRepair};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A mirror that heard no notice is still asked for its changes this often. Notices reach only the
/// sockets of the hub process that took the write, so this bounds how stale a mirror gets if the
/// hub ever runs as more than one process, or loses a notice to a bug.
pub const RECHECK: Duration = Duration::from_secs(10 * 60);
/// And read whole this often, which mends what a delta can't see: a database restored to an older
/// copy and written past the old head before this machine asked again.
pub const FULL_EVERY: Duration = Duration::from_secs(24 * 3600);

/// The hub calls a mirror needs: a sync's own, and the changes since a revision.
pub trait Source: sync::Hub {
    /// The changes to `project` since `since`, every file for None. `Ok(None)` when the hub
    /// predates revisions and answers only `list`.
    fn changes(&self, project: &str, since: Option<u64>) -> Result<Option<Changes>, String>;
}

/// What the socket thread tells the sync thread.
#[derive(Default)]
pub struct Watch {
    /// Notices heard per project since the link started. Only grows.
    heard: Mutex<HashMap<String, u64>>,
    /// Notices heard for any project. The sync loop wakes when it moves.
    notices: AtomicU64,
    /// Bumped on each accepted hello, before the connection carries a call.
    connection: AtomicU64,
}

impl Watch {
    /// The hub said `project` changed.
    pub fn heard(&self, project: &str) {
        *self.heard.lock().unwrap().entry(project.to_owned()).or_default() += 1;
        self.notices.fetch_add(1, Ordering::SeqCst);
    }

    /// The hub accepted a new connection, which has asked for no project's changes yet.
    pub fn connected(&self) {
        self.connection.fetch_add(1, Ordering::SeqCst);
    }

    pub fn notices(&self) -> u64 {
        self.notices.load(Ordering::SeqCst)
    }

    fn heard_for(&self, project: &str) -> u64 {
        self.heard.lock().unwrap().get(project).copied().unwrap_or_default()
    }

    fn connection(&self) -> u64 {
        self.connection.load(Ordering::SeqCst)
    }
}

/// Every project's mirror. Owned by the sync thread; nothing else writes it.
#[derive(Default)]
pub struct Mirrors {
    by_key: HashMap<String, Mirror>,
    /// What the hub of connection `.0` answers, learned on its first read, so a hub updated across
    /// a reconnect is asked again.
    hub: Option<(u64, Answers)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Answers {
    Changes,
    /// A hub from before revisions: `SyncChanges` is an unknown command there.
    ListOnly,
}

/// The hub's list of one project, current to `head`.
#[derive(Default)]
struct Mirror {
    entries: BTreeMap<String, Entry>,
    /// None until the first read, and after any read from a hub that answers only `list`.
    head: Option<u64>,
    /// The moment the last read was asked under. The mirror is stale once any of it moved.
    asked: Option<Moment>,
    full_at: Option<Instant>,
    /// This machine wrote to the project since `asked`. Set whatever the write answered: a write
    /// that failed or went unanswered may still have landed.
    wrote: bool,
}

/// What a read is asked under, taken before the call is sent.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Moment {
    heard: u64,
    connection: u64,
    at: Instant,
}

#[derive(Debug, PartialEq)]
enum Read {
    Full,
    Since(u64),
}

impl Mirror {
    /// Whether the mirror must be read before a sync trusts it at `now`, and how. From a hub that
    /// answers only `list`: once per cycle (`cycle` is when it began) and after this machine wrote.
    /// Otherwise whole with no head or past `FULL_EVERY`, and the changes since the head after a
    /// notice, a write, a new connection, or `RECHECK`.
    fn due(&self, now: Moment, answers: Option<Answers>, cycle: Instant) -> Option<Read> {
        let Some(asked) = self.asked else { return Some(Read::Full) };
        if answers == Some(Answers::ListOnly) {
            return (self.wrote || asked.at < cycle).then_some(Read::Full);
        }
        let Some(head) = self.head else { return Some(Read::Full) };
        if self.full_at.is_none_or(|at| now.at.saturating_duration_since(at) >= FULL_EVERY) {
            return Some(Read::Full);
        }
        let moved = self.wrote || now.heard != asked.heard || now.connection != asked.connection || now.at.saturating_duration_since(asked.at) >= RECHECK;
        moved.then_some(Read::Since(head))
    }

    /// Takes the hub's answer to a revision read. Taking the same answer twice leaves the same mirror.
    fn apply(&mut self, changes: Changes, asked: Moment) {
        if changes.full {
            self.entries.clear();
            self.full_at = Some(asked.at);
        }
        self.entries.extend(changes.entries.into_iter().map(|entry| (entry.path.clone(), entry)));
        self.head = Some(changes.head);
        self.asked = Some(asked);
        self.wrote = false;
    }

    /// Takes a list from a hub that answers only `list`. It names no revision, so the next
    /// revision read is full.
    fn replace(&mut self, listed: Vec<Entry>, asked: Moment) {
        self.entries = listed.into_iter().map(|entry| (entry.path.clone(), entry)).collect();
        self.head = None;
        self.full_at = None;
        self.asked = Some(asked);
        self.wrote = false;
    }
}

impl Read {
    fn since(&self) -> Option<u64> {
        match self {
            Read::Full => None,
            Read::Since(head) => Some(*head),
        }
    }
}

/// One cycle's hub. Lists come from the mirrors; writes go to the hub and leave the project's
/// mirror due. `sync::sync` and `sync::sync_plugins` take it as any other `sync::Hub`.
pub struct Feed<'a, S: Source> {
    source: &'a S,
    watch: &'a Watch,
    mirrors: RefCell<&'a mut Mirrors>,
    cycle: Instant,
}

impl<'a, S: Source> Feed<'a, S> {
    pub fn new(source: &'a S, watch: &'a Watch, mirrors: &'a mut Mirrors, cycle: Instant) -> Self {
        Feed { source, watch, mirrors: RefCell::new(mirrors), cycle }
    }

    fn wrote(&self, project: &str) {
        self.mirrors.borrow_mut().by_key.entry(project.to_owned()).or_default().wrote = true;
    }
}

impl<S: Source> sync::Hub for Feed<'_, S> {
    /// Every file the hub holds for the project, as `SyncList` answers. An error leaves the
    /// mirror as it was and fails the sync, as a failed list does.
    fn list(&self, project: &str) -> Result<Vec<Entry>, String> {
        // Taken before the call: a notice that lands while it is out leaves the mirror due again.
        let now = Moment { heard: self.watch.heard_for(project), connection: self.watch.connection(), at: Instant::now() };
        let mut mirrors = self.mirrors.borrow_mut();
        let Mirrors { by_key, hub } = &mut **mirrors;
        let answers = hub.filter(|(connection, _)| *connection == now.connection).map(|(_, answers)| answers);
        let mirror = by_key.entry(project.to_owned()).or_default();
        match mirror.due(now, answers, self.cycle) {
            None => {}
            Some(_) if answers == Some(Answers::ListOnly) => mirror.replace(self.source.list(project)?, now),
            Some(read) => match self.source.changes(project, read.since())? {
                Some(changes) => {
                    *hub = Some((now.connection, Answers::Changes));
                    mirror.apply(changes, now);
                }
                None => {
                    *hub = Some((now.connection, Answers::ListOnly));
                    mirror.replace(self.source.list(project)?, now);
                }
            },
        }
        Ok(mirror.entries.values().cloned().collect())
    }

    fn push(&self, project: &str, files: Vec<File>) -> Result<(), String> {
        let pushed = self.source.push(project, files);
        self.wrote(project);
        pushed
    }

    fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<File>, String> {
        self.source.pull(project, paths)
    }

    fn mark_executable(&self, project: &str, repairs: Vec<ModeRepair>) -> Result<(), String> {
        let marked = self.source.mark_executable(project, repairs);
        self.wrote(project);
        marked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::Hub;
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::UNIX_EPOCH;

    const KEY: &str = "github.com/acme/app";

    fn entry(path: &str, sha: &str) -> Entry {
        Entry { path: path.into(), sha: sha.into(), modified: 1, size: 1, executable: Some(false) }
    }

    /// A socket of one machine to `RevHub`: up, and whether it asked `changes` since it came up.
    #[derive(Clone, Copy, Default)]
    struct Socket {
        up: bool,
        asked: bool,
    }

    /// The hub's project rows with their revisions, as `db::sync_push` keeps them: a deletion
    /// whose `agreed` is stale is skipped, a history whose `base` is stale refuses the push, and a
    /// write that moved the head tells every machine whose socket asked `changes` since it came up.
    #[derive(Default)]
    struct RevHub {
        rows: RefCell<BTreeMap<String, (File, u64)>>,
        head: Cell<u64>,
        /// A hub from before revisions.
        old: bool,
        watches: Vec<Arc<Watch>>,
        sockets: RefCell<Vec<Socket>>,
        lists: Cell<usize>,
        reads: RefCell<Vec<Option<u64>>>,
        /// Entries the hub answered `list` and `changes` with.
        sent: Cell<usize>,
    }

    impl RevHub {
        fn with(machines: usize) -> RevHub {
            RevHub { watches: (0..machines).map(|_| Arc::default()).collect(), sockets: RefCell::new(vec![Socket { up: true, asked: false }; machines]), ..RevHub::default() }
        }

        fn entries(&self) -> Vec<Entry> {
            self.rows.borrow().values().map(|(file, _)| Entry { executable: Some(file.entry.executable.unwrap_or(false)), ..file.entry.clone() }).collect()
        }

        fn write(&self, file: File) {
            self.head.set(self.head.get() + 1);
            self.rows.borrow_mut().insert(file.entry.path.clone(), (file, self.head.get()));
        }

        /// Tells every socket that asked, the writer's too, once a write moved the head.
        fn tell(&self, before: u64) {
            if self.head.get() == before {
                return;
            }
            for (machine, socket) in self.sockets.borrow().iter().enumerate() {
                if socket.up && socket.asked {
                    self.watches[machine].heard(KEY);
                }
            }
        }

        fn drop_socket(&self, machine: usize) {
            self.sockets.borrow_mut()[machine] = Socket { up: false, asked: false };
        }

        /// The machine's link dials again when its socket dropped, as `link::run` does before
        /// the next cycle can call the hub.
        fn reconnect(&self, machine: usize) {
            let mut sockets = self.sockets.borrow_mut();
            if !sockets[machine].up {
                sockets[machine].up = true;
                self.watches[machine].connected();
            }
        }

        fn as_machine(&self, machine: usize) -> On<'_> {
            On { hub: self, machine }
        }
    }

    impl sync::Hub for RevHub {
        fn list(&self, _: &str) -> Result<Vec<Entry>, String> {
            self.lists.set(self.lists.get() + 1);
            let entries = self.entries();
            self.sent.set(self.sent.get() + entries.len());
            Ok(entries)
        }

        fn push(&self, _: &str, files: Vec<File>) -> Result<(), String> {
            let held = |path: &str| self.rows.borrow().get(path).map_or(String::new(), |(file, _)| file.entry.sha.clone());
            if files.iter().any(|f| f.base.as_ref().is_some_and(|base| *base != held(&f.entry.path))) {
                return Err(sync::STALE_REFUSAL.into());
            }
            let before = self.head.get();
            for file in files {
                let stale = file.entry.sha.is_empty() && file.agreed.as_ref().is_some_and(|agreed| self.rows.borrow().get(&file.entry.path).is_some_and(|(held, _)| held.entry.sha != *agreed));
                if !stale {
                    self.write(File { agreed: None, base: None, ..file });
                }
            }
            self.tell(before);
            Ok(())
        }

        fn pull(&self, _: &str, paths: Vec<String>) -> Result<Vec<File>, String> {
            let rows = self.rows.borrow();
            Ok(paths.iter().filter_map(|path| rows.get(path)).map(|(file, _)| File { data: file.data.clone().filter(|_| !file.entry.sha.is_empty()), agreed: None, base: None, ..file.clone() }).collect())
        }

        fn mark_executable(&self, _: &str, repairs: Vec<ModeRepair>) -> Result<(), String> {
            let before = self.head.get();
            for repair in repairs {
                let held = self.rows.borrow().get(&repair.path).filter(|(file, _)| file.entry.sha == repair.sha).map(|(file, _)| file.clone());
                if let Some(mut file) = held {
                    file.entry.executable = Some(true);
                    self.write(file);
                }
            }
            self.tell(before);
            Ok(())
        }
    }

    /// `RevHub` as one machine's link sees it.
    struct On<'a> {
        hub: &'a RevHub,
        machine: usize,
    }

    impl sync::Hub for On<'_> {
        fn list(&self, project: &str) -> Result<Vec<Entry>, String> {
            self.hub.list(project)
        }

        fn push(&self, project: &str, files: Vec<File>) -> Result<(), String> {
            self.hub.push(project, files)
        }

        fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<File>, String> {
            self.hub.pull(project, paths)
        }

        fn mark_executable(&self, project: &str, repairs: Vec<ModeRepair>) -> Result<(), String> {
            self.hub.mark_executable(project, repairs)
        }
    }

    impl Source for On<'_> {
        fn changes(&self, _: &str, since: Option<u64>) -> Result<Option<Changes>, String> {
            if !self.hub.sockets.borrow()[self.machine].up {
                return Err(crate::link::OFFLINE.into());
            }
            self.hub.reads.borrow_mut().push(since);
            if self.hub.old {
                return Ok(None);
            }
            self.hub.sockets.borrow_mut()[self.machine].asked = true;
            let head = self.hub.head.get();
            let full = since.is_none_or(|since| since > head);
            let entries: Vec<Entry> = self
                .hub
                .rows
                .borrow()
                .values()
                .filter(|(_, rev)| full || *rev > since.unwrap_or_default())
                .map(|(file, _)| Entry { executable: Some(file.entry.executable.unwrap_or(false)), ..file.entry.clone() })
                .collect();
            self.hub.sent.set(self.hub.sent.get() + entries.len());
            Ok(Some(Changes { head, full, entries }))
        }
    }

    fn file(path: &str, sha: &str) -> File {
        File { entry: entry(path, sha), data: Some(String::new()), agreed: None, base: None }
    }

    fn listed(feed: &impl sync::Hub) -> Vec<(String, String)> {
        feed.list(KEY).unwrap().into_iter().map(|e| (e.path, e.sha)).collect()
    }

    #[test]
    fn a_mirror_read_once_answers_the_next_lists_with_no_hub_call() {
        let hub = RevHub::with(1);
        hub.as_machine(0).push(KEY, vec![file("claude/a.jsonl", "a1")]).unwrap();
        let (on, mut mirrors) = (hub.as_machine(0), Mirrors::default());
        let feed = Feed::new(&on, &hub.watches[0], &mut mirrors, Instant::now());
        assert_eq!(listed(&feed), [("claude/a.jsonl".to_owned(), "a1".to_owned())]);
        assert_eq!(listed(&feed), [("claude/a.jsonl".to_owned(), "a1".to_owned())]);
        let feed = Feed::new(&on, &hub.watches[0], &mut mirrors, Instant::now());
        assert_eq!(listed(&feed), [("claude/a.jsonl".to_owned(), "a1".to_owned())]);
        assert_eq!(*hub.reads.borrow(), [None], "one full read, then nothing moved");
        assert_eq!(hub.lists.get(), 0);
    }

    #[test]
    fn a_notice_or_a_write_by_another_machine_brings_only_the_change() {
        let hub = RevHub::with(2);
        let (mac, vps) = (hub.as_machine(0), hub.as_machine(1));
        mac.push(KEY, vec![file("claude/a.jsonl", "a1"), file("claude/b.jsonl", "b1")]).unwrap();
        let mut mirrors = Mirrors::default();
        assert_eq!(listed(&Feed::new(&vps, &hub.watches[1], &mut mirrors, Instant::now())).len(), 2);
        mac.push(KEY, vec![file("claude/b.jsonl", "b2")]).unwrap();
        assert_eq!(hub.watches[1].notices(), 1, "the vps asked, so it hears of the mac's push");
        let feed = Feed::new(&vps, &hub.watches[1], &mut mirrors, Instant::now());
        assert_eq!(listed(&feed), [("claude/a.jsonl".to_owned(), "a1".to_owned()), ("claude/b.jsonl".to_owned(), "b2".to_owned())]);
        assert_eq!(*hub.reads.borrow(), [None, Some(2)]);
        assert_eq!(hub.sent.get(), 3, "the full read's two entries, then the one that changed");
    }

    /// A `Source` whose hub tells of a write while the read is out, after it read its rows.
    struct Racing<'a> {
        on: On<'a>,
    }

    impl sync::Hub for Racing<'_> {
        fn list(&self, project: &str) -> Result<Vec<Entry>, String> {
            self.on.list(project)
        }
        fn push(&self, project: &str, files: Vec<File>) -> Result<(), String> {
            self.on.push(project, files)
        }
        fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<File>, String> {
            self.on.pull(project, paths)
        }
        fn mark_executable(&self, project: &str, repairs: Vec<ModeRepair>) -> Result<(), String> {
            self.on.mark_executable(project, repairs)
        }
    }

    impl Source for Racing<'_> {
        fn changes(&self, project: &str, since: Option<u64>) -> Result<Option<Changes>, String> {
            let answer = self.on.changes(project, since);
            if since.is_none() {
                self.on.hub.as_machine(1).push(KEY, vec![file("claude/late.jsonl", "l1")]).unwrap();
            }
            answer
        }
    }

    #[test]
    fn a_notice_heard_while_a_read_is_out_makes_the_mirror_due_again() {
        let hub = RevHub::with(2);
        let racing = Racing { on: hub.as_machine(0) };
        let mut mirrors = Mirrors::default();
        let feed = Feed::new(&racing, &hub.watches[0], &mut mirrors, Instant::now());
        assert_eq!(listed(&feed), [], "the read saw the rows before the write");
        assert_eq!(listed(&feed), [("claude/late.jsonl".to_owned(), "l1".to_owned())]);
        assert_eq!(listed(&feed), [("claude/late.jsonl".to_owned(), "l1".to_owned())]);
        assert_eq!(*hub.reads.borrow(), [None, Some(0)]);
    }

    /// Fails every push, after the hub may have taken it.
    struct Unanswered<'a> {
        on: On<'a>,
    }

    impl sync::Hub for Unanswered<'_> {
        fn list(&self, project: &str) -> Result<Vec<Entry>, String> {
            self.on.list(project)
        }
        fn push(&self, project: &str, files: Vec<File>) -> Result<(), String> {
            self.on.push(project, files)?;
            Err(crate::link::HUB_TIMEOUT.into())
        }
        fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<File>, String> {
            self.on.pull(project, paths)
        }
        fn mark_executable(&self, project: &str, repairs: Vec<ModeRepair>) -> Result<(), String> {
            self.on.mark_executable(project, repairs)?;
            Err(crate::link::HUB_TIMEOUT.into())
        }
    }

    impl Source for Unanswered<'_> {
        fn changes(&self, project: &str, since: Option<u64>) -> Result<Option<Changes>, String> {
            self.on.changes(project, since)
        }
    }

    #[test]
    fn a_push_or_a_mark_makes_the_next_list_read_the_changes_even_when_it_errs() {
        let hub = RevHub::with(1);
        let source = Unanswered { on: hub.as_machine(0) };
        let mut mirrors = Mirrors::default();
        let feed = Feed::new(&source, &hub.watches[0], &mut mirrors, Instant::now());
        assert_eq!(listed(&feed), []);
        hub.sockets.borrow_mut()[0].asked = false;
        assert_eq!(feed.push(KEY, vec![file("claude/a.jsonl", "a1")]), Err(crate::link::HUB_TIMEOUT.into()));
        assert_eq!(listed(&feed), [("claude/a.jsonl".to_owned(), "a1".to_owned())], "no notice came, and the push landed");
        hub.sockets.borrow_mut()[0].asked = false;
        assert!(feed.mark_executable(KEY, vec![ModeRepair { path: "claude/a.jsonl".into(), sha: "a1".into() }]).is_err());
        assert_eq!(feed.list(KEY).unwrap()[0].executable, Some(true));
        assert_eq!(*hub.reads.borrow(), [None, Some(0), Some(1)]);
    }

    #[test]
    fn a_new_connection_makes_every_mirror_due() {
        let hub = RevHub::with(1);
        let on = hub.as_machine(0);
        let mut mirrors = Mirrors::default();
        let feed = Feed::new(&on, &hub.watches[0], &mut mirrors, Instant::now());
        feed.list(KEY).unwrap();
        feed.list("github.com/acme/site").unwrap();
        hub.drop_socket(0);
        hub.reconnect(0);
        feed.list(KEY).unwrap();
        feed.list("github.com/acme/site").unwrap();
        feed.list(KEY).unwrap();
        assert_eq!(*hub.reads.borrow(), [None, None, Some(0), Some(0)]);
    }

    #[test]
    fn an_old_hub_is_listed_once_per_project_per_cycle_and_after_a_write() {
        let hub = RevHub { old: true, ..RevHub::with(1) };
        let on = hub.as_machine(0);
        let mut mirrors = Mirrors::default();
        let feed = Feed::new(&on, &hub.watches[0], &mut mirrors, Instant::now());
        feed.list(KEY).unwrap();
        feed.list(KEY).unwrap();
        assert_eq!(hub.lists.get(), 1, "a plugin share's second list in a cycle is answered here");
        feed.push(KEY, vec![file("claude/a.jsonl", "a1")]).unwrap();
        assert_eq!(listed(&feed), [("claude/a.jsonl".to_owned(), "a1".to_owned())]);
        assert_eq!(hub.lists.get(), 2);
        std::thread::sleep(Duration::from_millis(1));
        let feed = Feed::new(&on, &hub.watches[0], &mut mirrors, Instant::now());
        feed.list(KEY).unwrap();
        feed.list(KEY).unwrap();
        assert_eq!(hub.lists.get(), 3, "the next cycle lists again");
        assert_eq!(*hub.reads.borrow(), [None], "the old hub was asked for changes once, on this connection");
    }

    #[test]
    fn a_full_answer_replaces_the_mirror_and_keeps_tombstones_and_one_answer_twice_changes_nothing() {
        let at = Moment { heard: 0, connection: 0, at: Instant::now() };
        let mut mirror = Mirror::default();
        mirror.apply(Changes { head: 3, full: true, entries: vec![entry("a", "1"), entry("b", "2")] }, at);
        mirror.apply(Changes { head: 5, full: false, entries: vec![entry("b", ""), entry("c", "3")] }, at);
        let shas = |mirror: &Mirror| mirror.entries.values().map(|e| (e.path.clone(), e.sha.clone())).collect::<Vec<_>>();
        let pairs = |list: &[(&str, &str)]| list.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect::<Vec<_>>();
        assert_eq!(shas(&mirror), pairs(&[("a", "1"), ("b", ""), ("c", "3")]), "a deletion stays as a tombstone, as the hub lists it");
        mirror.apply(Changes { head: 5, full: false, entries: vec![entry("b", ""), entry("c", "3")] }, at);
        assert_eq!(shas(&mirror), pairs(&[("a", "1"), ("b", ""), ("c", "3")]));
        mirror.apply(Changes { head: 2, full: true, entries: vec![entry("z", "9")] }, at);
        assert_eq!(shas(&mirror), pairs(&[("z", "9")]), "a head the hub went back past is read whole");
        assert_eq!(mirror.head, Some(2));
    }

    #[test]
    fn the_recheck_and_the_daily_full_read_fall_due_at_their_times() {
        let start = Instant::now();
        let at = |secs: u64| Moment { heard: 0, connection: 0, at: start + Duration::from_secs(secs) };
        let mut mirror = Mirror::default();
        assert_eq!(mirror.due(at(0), None, start), Some(Read::Full));
        mirror.apply(Changes { head: 7, full: true, entries: vec![] }, at(0));
        assert_eq!(mirror.due(at(599), Some(Answers::Changes), start), None);
        assert_eq!(mirror.due(at(600), Some(Answers::Changes), start), Some(Read::Since(7)));
        mirror.apply(Changes { head: 7, full: false, entries: vec![] }, at(600));
        assert_eq!(mirror.due(at(1199), Some(Answers::Changes), start), None, "an empty recheck counts as a read");
        assert_eq!(mirror.due(Moment { heard: 1, ..at(601) }, Some(Answers::Changes), start), Some(Read::Since(7)));
        assert_eq!(mirror.due(at(24 * 3600 - 1), Some(Answers::Changes), start), Some(Read::Since(7)));
        assert_eq!(mirror.due(at(24 * 3600), Some(Answers::Changes), start), Some(Read::Full));
    }

    /// One machine of the convergence run: a checkout of `KEY` in a folder of its own.
    struct Machine {
        checkout: sync::Checkout,
        state: sync::State,
        mirrors: Mirrors,
    }

    impl Machine {
        fn new(root: &Path, name: &str) -> Machine {
            let home = root.join(name);
            let path = home.join("app");
            std::fs::create_dir_all(&path).unwrap();
            let project = sync::Project { key: KEY.into(), name: "app".into(), repo: "git@github.com:acme/app.git".into() };
            let checkout = sync::Checkout { project, path, history: Some(home.join("history/app.jsonl")), claude_home: Some(home.join(".claude")) };
            Machine { checkout, state: sync::State::default(), mirrors: Mirrors::default() }
        }

        fn local(&self, path: &str) -> PathBuf {
            match path.strip_prefix("claude/") {
                Some(rest) => sync::claude_dir(self.checkout.claude_home.as_ref().unwrap(), &self.checkout.path).join(rest),
                None if path == sync::HISTORY => self.checkout.history.clone().unwrap(),
                None => self.checkout.path.join(".claude").join(path),
            }
        }

        /// Every synced file here by its name on the hub, with its bytes.
        fn files(&self) -> BTreeMap<String, Vec<u8>> {
            PATHS.iter().filter_map(|path| std::fs::read(self.local(path)).ok().map(|bytes| (path.to_string(), bytes))).collect()
        }
    }

    const PATHS: &[&str] = &["claude/s1.jsonl", "claude/s2.jsonl", "claude/memory/a.md", "claude/memory/b.md", "skills/tidy/SKILL.md", sync::HISTORY];

    /// xorshift64*, so a run's events repeat exactly.
    struct Draw(u64);

    impl Draw {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) % n
        }
    }

    fn write_at(target: &Path, bytes: &[u8], millis: u64) {
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, bytes).unwrap();
        std::fs::File::options().write(true).open(target).unwrap().set_modified(UNIX_EPOCH + Duration::from_millis(millis)).unwrap();
    }

    /// Three machines edit, delete and sync one project in an order `seed` draws. With `feed`, each
    /// syncs through its `Feed` and the hub tells of writes; sockets drop, losing their notices,
    /// and dial again before the machine's next cycle, as a link does. The hub restarts, dropping
    /// every socket, and a machine's app restarts, dropping its mirrors. Without `feed`, each lists
    /// the hub every sync, as before. Returns each machine's files and what it agreed, after every
    /// machine synced three more times with no edit.
    fn converge(root: &Path, seed: u64, feed: bool, old_hub: bool) -> (Vec<(BTreeMap<String, Vec<u8>>, serde_json::Value)>, usize) {
        let _ = std::fs::remove_dir_all(root);
        let hub = RevHub { old: old_hub, ..RevHub::with(3) };
        let mut machines: Vec<Machine> = ["mac", "vps", "mini"].iter().map(|name| Machine::new(root, name)).collect();
        let mut draw = Draw(seed);
        let start = 1_763_000_000_000u64;
        let sync = |machine: &mut Machine, index: usize, millis: u64| {
            let now = UNIX_EPOCH + Duration::from_millis(millis);
            if !feed {
                return sync::sync(&hub, &mut machine.state, &machine.checkout, now);
            }
            hub.reconnect(index);
            let on = hub.as_machine(index);
            let feed = Feed::new(&on, &hub.watches[index], &mut machine.mirrors, Instant::now());
            let listed: BTreeMap<String, Entry> = feed.list(KEY)?.into_iter().map(|e| (e.path.clone(), e)).collect();
            let held: BTreeMap<String, Entry> = hub.entries().into_iter().map(|e| (e.path.clone(), e)).collect();
            assert_eq!(listed, held, "seed {seed}: the mirror is the hub's list when a sync begins");
            sync::sync(&feed, &mut machine.state, &machine.checkout, now)
        };
        for step in 0..400u64 {
            let millis = start + step * 1000;
            let index = draw.below(3) as usize;
            let machine = &mut machines[index];
            match draw.below(10) {
                0..=2 => {
                    let path = PATHS[draw.below(PATHS.len() as u64) as usize];
                    let target = machine.local(path);
                    let mut bytes = std::fs::read(&target).unwrap_or_default();
                    bytes.extend(format!("{{\"ended\":{step},\"by\":{index}}}\n").as_bytes());
                    write_at(&target, &bytes, millis);
                }
                3 => {
                    let path = PATHS[draw.below(PATHS.len() as u64) as usize];
                    let _ = std::fs::remove_file(machine.local(path));
                }
                4..=7 => {
                    sync(machine, index, millis).unwrap();
                }
                8 => match (feed, draw.below(3)) {
                    (false, _) => {}
                    (true, 0) => hub.drop_socket(index),
                    (true, 1) => (0..3).for_each(|i| hub.drop_socket(i)),
                    (true, _) => machine.mirrors = Mirrors::default(),
                },
                _ => {}
            }
        }
        let end = start + 400 * 1000;
        for round in 0..3 {
            for (index, machine) in machines.iter_mut().enumerate() {
                sync(machine, index, end + round * 1000).unwrap();
            }
        }
        let held = machines.iter().map(|m| (m.files(), serde_json::to_value(&m.state).unwrap()["agreed"].clone())).collect();
        (held, hub.sent.get())
    }

    /// Requirement 3 of the design: a feed that misses notices with its socket ends where today's
    /// list-every-sync loop ends, byte for byte, and sends far fewer entries.
    #[test]
    fn a_feed_with_dropped_notices_ends_where_listing_every_sync_ends() {
        let root = std::env::temp_dir().join(format!("pinned-mirror-{}-converge", std::process::id()));
        for seed in [7, 11, 2026, 90210] {
            let (listing, listed) = converge(&root, seed, false, false);
            let (fed, sent) = converge(&root, seed, true, false);
            assert_eq!(fed, listing, "seed {seed}");
            assert!(listing.iter().all(|(files, _)| files == &listing[0].0), "seed {seed}: the three machines hold the same files");
            assert!(sent < listed, "seed {seed}: the feed was sent {sent} entries, listing {listed}");
            eprintln!("seed {seed}: entries sent {sent} by feed, {listed} by listing");
            let (old, _) = converge(&root, seed, true, true);
            assert_eq!(old, listing, "seed {seed}: against a hub from before revisions");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_hub_tells_a_machine_only_after_it_asked_on_this_connection() {
        let hub = RevHub::with(2);
        hub.as_machine(1).push(KEY, vec![file("claude/a.jsonl", "a1")]).unwrap();
        assert_eq!(hub.watches[0].notices(), 0, "it never asked");
        let mut mirrors = Mirrors::default();
        Feed::new(&hub.as_machine(0), &hub.watches[0], &mut mirrors, Instant::now()).list(KEY).unwrap();
        hub.as_machine(1).push(KEY, vec![file("claude/a.jsonl", "a2")]).unwrap();
        assert_eq!(hub.watches[0].notices(), 1);
        hub.drop_socket(0);
        hub.as_machine(1).push(KEY, vec![file("claude/a.jsonl", "a3")]).unwrap();
        assert_eq!(hub.watches[0].notices(), 1, "lost with its socket");
    }
}
