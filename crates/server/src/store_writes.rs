//! The writes of one account's row behind `PgStorage`: the store document (workflows and runs)
//! and the connector keys. Each kind keeps one slot that holds only its newest document, so a
//! write that stalls holds at most one waiting document of each kind, never a queue of them.
//!
//! Every document takes a sequence number when it is queued, under the lock of the state it was
//! read from, so a higher number is a newer state. A slot keeps its document until a write of
//! it succeeds; a newer document replaces it in place. The writer writes only what a slot holds
//! at the time, so the numbers it writes only grow and an older document never lands after a
//! newer one, across failures and retries too.
//!
//! The number is also the version the row keeps with the document, and a write lands only above
//! the version stored: a write the hub gave up on that Postgres runs late never replaces a newer
//! one. The numbers start above the versions the row held when the account was loaded, so a new
//! hub process writes above what an earlier one wrote.
//!
//! Two hub processes write one row while a deploy runs both. A write that finds a higher version
//! in the row, which the other process wrote, moves this process's numbers above it and writes
//! the slot's document again as the next number: whole documents, no merge, and the one that
//! lands last wins. Only the slot's document, the newest, is renumbered, so an older document of
//! this process still never lands after a newer one. A document is never let go for being
//! overtaken: after `RENUMBER_AT_MOST` renumbers in a row the writer waits out a backoff, as after
//! a failed write, and renumbers it again.
pub(crate) use crate::db::WriteFailure;
use futures_util::future::BoxFuture;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use uuid::Uuid;

/// The first wait after a failed write; it doubles up to `RETRY_AT_MOST`.
const RETRY_FIRST: Duration = Duration::from_millis(500);
const RETRY_AT_MOST: Duration = Duration::from_secs(10);
/// How often `flush` asks a writer that waits out a failure to try again.
const HURRY_EVERY: Duration = Duration::from_millis(250);
/// How many times in a row one document is renumbered above a version another process wrote
/// before the writer waits out a backoff, as after a failed write, and renumbers it again.
const RENUMBER_AT_MOST: u32 = 3;

/// A document as the writer hands it to the database.
pub(crate) enum Doc {
    Store(Arc<StoreBytes>),
    Secrets(BTreeMap<String, String>),
}

/// Writes one document of the account to its row as the version given.
pub(crate) type Write = Arc<dyn Fn(Uuid, u64, Doc) -> BoxFuture<'static, Result<Landed, WriteFailure>> + Send + Sync>;

/// What a write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Landed {
    Written,
    /// Not written: the row holds this version of the kind, the version given or a newer one.
    Behind(u64),
}

/// The store and the runs as the state held them, JSON text that `save` and `save_runs` read
/// through; the database joins them into the row's one document.
pub(crate) struct StoreBytes {
    pub(crate) store: Option<Vec<u8>>,
    pub(crate) runs: Vec<u8>,
}

struct Queued<T> {
    seq: u64,
    doc: Arc<T>,
    bytes: usize,
    /// How often the document was numbered again above a version another process wrote since
    /// the last backoff.
    renumbered: u32,
}

#[derive(Default)]
struct Slots {
    next: u64,
    store: Option<Queued<StoreBytes>>,
    secrets: Option<Queued<BTreeMap<String, String>>>,
    /// The newest number written of each kind.
    written: (u64, u64),
    closed: bool,
}

/// What waits to be written, as the census counts it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub store: usize,
    pub store_bytes: usize,
    pub secrets: usize,
    pub secrets_bytes: usize,
}

pub(crate) struct Writes {
    account: Uuid,
    slots: Mutex<Slots>,
    /// A document was queued, or the writes were closed.
    queued: Notify,
    /// A slot emptied; `flush` waits on it.
    emptied: Notify,
    /// `flush` asks a writer that waits to retry to retry now.
    hurry: Notify,
    write: Write,
}

#[derive(Clone, Copy)]
enum Kind {
    Store,
    Secrets,
}

impl Writes {
    /// The account's slots and the task that writes them, which ends once `close` ran and
    /// nothing waits. The documents take numbers above `stored`, the newest version the row held.
    pub(crate) fn start(account: Uuid, stored: u64, write: Write) -> Arc<Writes> {
        let slots = Slots { next: stored, written: (stored, stored), ..Slots::default() };
        let writes = Arc::new(Writes { account, slots: Mutex::new(slots), queued: Notify::new(), emptied: Notify::new(), hurry: Notify::new(), write });
        tokio::spawn(writes.clone().run());
        writes
    }

    /// Queues the store and the runs as the account's newest document. The caller holds the lock
    /// of the state they come from, so the numbers follow the order the state changed in.
    pub(crate) fn queue_store(&self, store: Option<Vec<u8>>, runs: Vec<u8>) -> u64 {
        let bytes = store.as_ref().map_or(0, Vec::len) + runs.len();
        let mut slots = self.slots.lock().unwrap();
        slots.next += 1;
        let seq = slots.next;
        slots.store = Some(Queued { seq, doc: Arc::new(StoreBytes { store, runs }), bytes, renumbered: 0 });
        drop(slots);
        self.queued.notify_one();
        seq
    }

    /// Queues the connector keys, under the keys' lock like `queue_store`.
    pub(crate) fn queue_secrets(&self, keys: BTreeMap<String, String>) -> u64 {
        let bytes = keys.iter().map(|(name, key)| name.len() + key.len()).sum();
        let mut slots = self.slots.lock().unwrap();
        slots.next += 1;
        let seq = slots.next;
        slots.secrets = Some(Queued { seq, doc: Arc::new(keys), bytes, renumbered: 0 });
        drop(slots);
        self.queued.notify_one();
        seq
    }

    pub fn pending(&self) -> Pending {
        let slots = self.slots.lock().unwrap();
        Pending {
            store: slots.store.is_some() as usize,
            store_bytes: slots.store.as_ref().map_or(0, |q| q.bytes),
            secrets: slots.secrets.is_some() as usize,
            secrets_bytes: slots.secrets.as_ref().map_or(0, |q| q.bytes),
        }
    }

    /// Writes what waits now, giving up at `within`. True when nothing waits any more.
    pub async fn flush(&self, within: Duration) -> bool {
        let until = tokio::time::Instant::now() + within;
        loop {
            let emptied = self.emptied.notified();
            tokio::pin!(emptied);
            emptied.as_mut().enable();
            if self.pending() == Pending::default() {
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= until {
                return false;
            }
            // A writer waiting out a failure tries again now, and again every `HURRY_EVERY`. Only a
            // writer waiting now: a permit kept for later would skip a backoff after the flush.
            self.hurry.notify_waiters();
            let _ = tokio::time::timeout((until - now).min(HURRY_EVERY), emptied).await;
        }
    }

    /// No more documents come; the writer ends once it wrote what waits.
    pub(crate) fn close(&self) {
        self.slots.lock().unwrap().closed = true;
        self.queued.notify_one();
    }

    /// The newest document of `kind` with its number, or none when it was written.
    fn take(&self, kind: Kind) -> Option<(u64, Doc)> {
        let (seq, doc) = {
            let slots = self.slots.lock().unwrap();
            match kind {
                Kind::Store => slots.store.as_ref().map(|q| (q.seq, Err(q.doc.clone())))?,
                Kind::Secrets => slots.secrets.as_ref().map(|q| (q.seq, Ok(q.doc.clone())))?,
            }
        };
        let doc = match doc {
            Ok(keys) => Doc::Secrets((*keys).clone()),
            Err(bytes) => Doc::Store(bytes),
        };
        Some((seq, doc))
    }

    /// Empties `kind`'s slot when it still holds `seq`, the document just written or one Postgres
    /// refused.
    fn written(&self, kind: Kind, seq: u64) {
        let mut slots = self.slots.lock().unwrap();
        let (slot_seq, written) = match kind {
            Kind::Store => (slots.store.as_ref().map(|q| q.seq), &mut slots.written.0),
            Kind::Secrets => (slots.secrets.as_ref().map(|q| q.seq), &mut slots.written.1),
        };
        *written = (*written).max(seq);
        if slot_seq == Some(seq) {
            match kind {
                Kind::Store => slots.store = None,
                Kind::Secrets => slots.secrets = None,
            }
        }
        drop(slots);
        self.emptied.notify_waiters();
    }

    /// The write of `seq` found `stored` in the row, which another process wrote. Moves the numbers
    /// above it and, when the slot's document is not above it yet, numbers that one again so the
    /// writer writes it next: the document of `seq` itself, or a newer one that replaced it.
    /// The renumber after `RENUMBER_AT_MOST` in a row answers true: the writer waits out a backoff
    /// before it writes the document again, and the count starts over.
    fn behind(&self, kind: Kind, seq: u64, stored: u64, what: &str) -> bool {
        fn renumber<T>(slot: &mut Option<Queued<T>>, next: &mut u64, stored: u64) -> Option<(u64, bool)> {
            *next = (*next).max(stored);
            let queued = slot.as_mut().filter(|q| q.seq <= stored)?;
            *next += 1;
            queued.seq = *next;
            let wait = queued.renumbered >= RENUMBER_AT_MOST;
            queued.renumbered = if wait { 0 } else { queued.renumbered + 1 };
            Some((*next, wait))
        }
        // The row holds `seq` at least, whatever a read that raced the write answered.
        let stored = stored.max(seq);
        let mut slots = self.slots.lock().unwrap();
        let Slots { next, store, secrets, .. } = &mut *slots;
        let renumbered = match kind {
            Kind::Store => renumber(store, next, stored),
            Kind::Secrets => renumber(secrets, next, stored),
        };
        drop(slots);
        match renumbered {
            Some((again, false)) => eprintln!("pinned-server: account {}'s {what} version {seq} not written: the row holds version {stored}; writing it as version {again}", self.account),
            Some((again, true)) => {
                eprintln!("pinned-server: account {}'s {what} version {seq} not written: the row holds version {stored}, renumbered {RENUMBER_AT_MOST} times in a row; writing it as version {again} after a backoff", self.account);
                return true;
            }
            None => {}
        }
        false
    }

    async fn run(self: Arc<Self>) {
        let mut failures = 0u32;
        loop {
            let queued = self.queued.notified();
            tokio::pin!(queued);
            queued.as_mut().enable();
            let (mut failed, mut renumbered) = (false, false);
            for kind in [Kind::Store, Kind::Secrets] {
                let Some((seq, doc)) = self.take(kind) else { continue };
                debug_assert!(seq > self.written_seq(kind), "a document older than one written");
                let what = match kind {
                    Kind::Store => "workflows",
                    Kind::Secrets => "connector keys",
                };
                match (self.write)(self.account, seq, doc).await {
                    Ok(Landed::Written) => self.written(kind, seq),
                    Ok(Landed::Behind(stored)) => {
                        renumbered = true;
                        failed |= self.behind(kind, seq, stored, what);
                    }
                    Err(WriteFailure::Refused { sqlstate }) => {
                        // Writing it again is refused again; a newer document may still be written.
                        eprintln!("pinned-server: Postgres refused account {}'s {what} version {seq} (SQLSTATE {sqlstate}), dropped", self.account);
                        self.written(kind, seq);
                    }
                    Err(WriteFailure::Failed(e)) => {
                        failed = true;
                        eprintln!("pinned-server: could not save account {}'s {what}, retrying: {}", self.account, short(&e));
                    }
                }
            }
            if failed {
                failures += 1;
                let wait = RETRY_FIRST.saturating_mul(1 << (failures - 1).min(10)).min(RETRY_AT_MOST);
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = self.hurry.notified() => {}
                }
                continue;
            }
            // Renumbers between backoffs keep the backoff growing until a write lands.
            if !renumbered {
                failures = 0;
            }
            {
                let slots = self.slots.lock().unwrap();
                if slots.store.is_some() || slots.secrets.is_some() {
                    continue;
                }
                if slots.closed {
                    return;
                }
            }
            queued.await;
        }
    }

    fn written_seq(&self, kind: Kind) -> u64 {
        let slots = self.slots.lock().unwrap();
        match kind {
            Kind::Store => slots.written.0,
            Kind::Secrets => slots.written.1,
        }
    }
}

/// Flushes the writes of every account `accounts` names, asking for them again after each pass so
/// a document saved meanwhile, or an account loaded meanwhile, is written too. Ends once a pass
/// finds nothing waiting, or at `within`. Answers how many accounts had something waiting and
/// the ones still waiting at the end. A save after the pass that found nothing waiting is not
/// written.
pub(crate) async fn flush_all(accounts: impl Fn() -> Vec<(Uuid, Arc<Writes>)>, within: Duration) -> (usize, Vec<Uuid>) {
    let until = tokio::time::Instant::now() + within;
    let mut waited = std::collections::HashSet::new();
    loop {
        let waiting: Vec<(Uuid, Arc<Writes>)> = accounts().into_iter().filter(|(_, w)| w.pending() != Pending::default()).collect();
        waited.extend(waiting.iter().map(|(id, _)| *id));
        let left = until.saturating_duration_since(tokio::time::Instant::now());
        if waiting.is_empty() || left.is_zero() {
            return (waited.len(), waiting.into_iter().map(|(id, _)| id).collect());
        }
        futures_util::future::join_all(waiting.iter().map(|(_, w)| w.flush(left))).await;
    }
}

/// A failure's first line, cut short, for a log line.
fn short(e: &str) -> String {
    let line = e.lines().next().unwrap_or_default();
    line.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Semaphore;

    /// A database whose writes wait for a permit each, and that can fail the next ones.
    #[derive(Default)]
    struct Fake {
        written: Mutex<Vec<Value>>,
        keys: Mutex<Vec<BTreeMap<String, String>>>,
        /// The version each write carried, in order.
        versions: Mutex<Vec<u64>>,
        /// The versions the row holds of the store and of the keys. A write lands only above, as
        /// in Postgres; writers that share the fake share the row, as two hub processes do.
        row: Mutex<(u64, u64)>,
        /// The next this many writes find that another process wrote 5 versions more meanwhile.
        overtakes: AtomicUsize,
        gate: Option<Arc<Semaphore>>,
        fail: AtomicUsize,
        /// Writes that land but answer a failure, as a statement the hub gave up on that
        /// Postgres runs anyway.
        lost_answers: AtomicUsize,
        tried: AtomicUsize,
    }

    /// The row's document, as the database joins it.
    fn joined(bytes: &StoreBytes) -> Value {
        let mut doc: Value = bytes.store.as_ref().map_or(serde_json::json!({}), |store| serde_json::from_slice(store).unwrap());
        let object = doc.as_object_mut().unwrap();
        object.remove("runs");
        if !bytes.runs.is_empty() {
            object.insert("runs".to_owned(), serde_json::from_slice(&bytes.runs).unwrap());
        }
        doc
    }

    fn writes(fake: &Arc<Fake>) -> Arc<Writes> {
        writes_above(fake, 0)
    }

    fn writes_above(fake: &Arc<Fake>, stored: u64) -> Arc<Writes> {
        let fake = fake.clone();
        Writes::start(
            Uuid::nil(),
            stored,
            Arc::new(move |_, version, doc| {
                let fake = fake.clone();
                Box::pin(async move {
                    if let Some(gate) = &fake.gate {
                        gate.acquire().await.unwrap().forget();
                    }
                    fake.tried.fetch_add(1, Ordering::SeqCst);
                    if let Doc::Store(bytes) = &doc {
                        if bytes.store.as_deref().is_some_and(|store| store.windows(6).any(|w| w == br"\u0000")) {
                            return Err(WriteFailure::Refused { sqlstate: "22P05".into() });
                        }
                    }
                    if fake.fail.load(Ordering::SeqCst) > 0 {
                        fake.fail.fetch_sub(1, Ordering::SeqCst);
                        return Err(WriteFailure::Failed("connection reset".into()));
                    }
                    {
                        let mut row = fake.row.lock().unwrap();
                        let held = match &doc {
                            Doc::Store(_) => &mut row.0,
                            Doc::Secrets(_) => &mut row.1,
                        };
                        if fake.overtakes.load(Ordering::SeqCst) > 0 {
                            fake.overtakes.fetch_sub(1, Ordering::SeqCst);
                            *held += 5;
                        }
                        if version <= *held {
                            return Ok(Landed::Behind(*held));
                        }
                        *held = version;
                    }
                    fake.versions.lock().unwrap().push(version);
                    match doc {
                        Doc::Store(bytes) => fake.written.lock().unwrap().push(joined(&bytes)),
                        Doc::Secrets(k) => fake.keys.lock().unwrap().push(k),
                    }
                    if fake.lost_answers.load(Ordering::SeqCst) > 0 {
                        fake.lost_answers.fetch_sub(1, Ordering::SeqCst);
                        return Err(WriteFailure::Failed("Postgres did not answer; the connection is given up".into()));
                    }
                    Ok(Landed::Written)
                })
            }),
        )
    }

    fn store(n: u32) -> Option<Vec<u8>> {
        Some(format!(r#"{{"n":{n}}}"#).into_bytes())
    }

    async fn settle() {
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn a_blocked_writer_keeps_one_pending_document_the_newest_and_writes_it_once_released() {
        let gate = Arc::new(Semaphore::new(0));
        let fake = Arc::new(Fake { gate: Some(gate.clone()), ..Fake::default() });
        let writes = writes(&fake);
        for n in 1..=50 {
            writes.queue_store(store(n), br#"[{"id":"r"}]"#.to_vec());
            settle().await;
        }
        let pending = writes.pending();
        assert_eq!(pending.store, 1, "one document waits, however many saves came");
        assert_eq!(pending.store_bytes, store(50).unwrap().len() + br#"[{"id":"r"}]"#.len(), "and it is the newest");
        // The first write took document 1 before the gate; release it and the next one.
        gate.add_permits(10);
        assert!(writes.flush(Duration::from_secs(5)).await);
        let written = fake.written.lock().unwrap().clone();
        assert_eq!(written.last().unwrap(), &serde_json::json!({ "n": 50, "runs": [{ "id": "r" }] }));
        assert!(written.len() <= 2, "a burst is at most the write in flight and the newest: {written:?}");
        let ns: Vec<u64> = written.iter().map(|v| v["n"].as_u64().unwrap()).collect();
        assert!(ns.windows(2).all(|w| w[0] < w[1]), "an older document never lands after a newer one: {ns:?}");
    }

    #[tokio::test]
    async fn store_and_keys_each_keep_their_own_newest_document() {
        let gate = Arc::new(Semaphore::new(0));
        let fake = Arc::new(Fake { gate: Some(gate.clone()), ..Fake::default() });
        let writes = writes(&fake);
        for n in 1..=5 {
            writes.queue_store(store(n), vec![]);
            writes.queue_secrets(BTreeMap::from([("k".to_owned(), format!("v{n}"))]));
            settle().await;
        }
        let pending = writes.pending();
        assert_eq!((pending.store, pending.secrets), (1, 1));
        gate.add_permits(10);
        assert!(writes.flush(Duration::from_secs(5)).await);
        assert_eq!(fake.written.lock().unwrap().last().unwrap(), &serde_json::json!({ "n": 5 }));
        assert_eq!(fake.keys.lock().unwrap().last().unwrap()["k"], "v5");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_write_is_retried_and_a_newer_save_meanwhile_is_what_lands() {
        let fake = Arc::new(Fake { fail: AtomicUsize::new(3), ..Fake::default() });
        let writes = writes(&fake);
        writes.queue_store(store(1), vec![]);
        settle().await;
        assert_eq!(fake.tried.load(Ordering::SeqCst), 1);
        assert_eq!(writes.pending().store, 1, "a failed document stays pending");
        writes.queue_store(store(2), vec![]);
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert_eq!(writes.pending(), Pending::default());
        assert_eq!(*fake.written.lock().unwrap(), vec![serde_json::json!({ "n": 2 })], "document 1 never lands after document 2");
        assert!(fake.tried.load(Ordering::SeqCst) >= 4);
    }

    #[tokio::test(start_paused = true)]
    async fn a_flush_retries_at_once_and_writes_what_waits() {
        let fake = Arc::new(Fake { fail: AtomicUsize::new(4), ..Fake::default() });
        let writes = writes(&fake);
        writes.queue_store(store(7), vec![]);
        writes.queue_secrets(BTreeMap::from([("k".to_owned(), "v".to_owned())]));
        let started = tokio::time::Instant::now();
        assert!(writes.flush(Duration::from_secs(10)).await, "the flush wrote both documents");
        assert!(started.elapsed() < Duration::from_secs(2), "the flush did not wait out the backoff: {:?}", started.elapsed());
        assert_eq!(*fake.written.lock().unwrap(), vec![serde_json::json!({ "n": 7 })]);
        assert_eq!(fake.keys.lock().unwrap().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_flush_gives_up_at_its_limit_while_the_database_is_down() {
        let gate = Arc::new(Semaphore::new(0));
        let fake = Arc::new(Fake { gate: Some(gate), ..Fake::default() });
        let writes = writes(&fake);
        writes.queue_store(store(1), vec![]);
        assert!(!writes.flush(Duration::from_secs(10)).await);
        assert_eq!(writes.pending().store, 1);
    }

    #[tokio::test]
    async fn a_closed_writer_still_writes_what_waits_then_ends() {
        let fake = Arc::new(Fake::default());
        let writes = writes(&fake);
        writes.queue_store(store(3), vec![]);
        writes.close();
        assert!(writes.flush(Duration::from_secs(5)).await);
        settle().await;
        assert_eq!(Arc::strong_count(&writes), 1, "the writer task ended and let go of the slots");
        assert_eq!(fake.written.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_versions_start_above_what_the_row_held_and_grow() {
        let fake = Arc::new(Fake::default());
        let writes = writes_above(&fake, 41);
        writes.queue_store(store(1), vec![]);
        assert!(writes.flush(Duration::from_secs(5)).await);
        writes.queue_secrets(BTreeMap::from([("k".to_owned(), "v".to_owned())]));
        assert!(writes.flush(Duration::from_secs(5)).await);
        assert_eq!(*fake.versions.lock().unwrap(), vec![42, 43], "a new process writes above the stored version");
    }

    /// Two hub processes, as a deploy runs them, load the account at the same version. The first
    /// saves many times; the second, numbered lower, saves once after, and its save lands.
    #[tokio::test]
    async fn a_save_of_the_lower_numbered_of_two_processes_on_one_row_lands_above_the_other() {
        let fake = Arc::new(Fake { row: Mutex::new((10, 10)), ..Fake::default() });
        let (first, second) = (writes_above(&fake, 10), writes_above(&fake, 10));
        for n in 1..=20 {
            first.queue_store(store(n), vec![]);
            first.queue_secrets(BTreeMap::from([("k".to_owned(), format!("first {n}"))]));
            assert!(first.flush(Duration::from_secs(5)).await);
        }
        assert_eq!(*fake.row.lock().unwrap(), (49, 50));

        second.queue_store(store(100), vec![]);
        second.queue_secrets(BTreeMap::from([("k".to_owned(), "second".to_owned())]));
        assert!(second.flush(Duration::from_secs(5)).await);
        assert_eq!(fake.written.lock().unwrap().last().unwrap(), &serde_json::json!({ "n": 100 }), "the second process's later store is in the row");
        assert_eq!(fake.keys.lock().unwrap().last().unwrap()["k"], "second", "and its later keys");
        assert_eq!(*fake.row.lock().unwrap(), (50, 51), "numbered above what the first wrote");

        // The first saves again, numbered above the row or renumbered above it: the write that
        // lands last wins.
        first.queue_store(store(21), vec![]);
        first.queue_secrets(BTreeMap::from([("k".to_owned(), "first again".to_owned())]));
        assert!(first.flush(Duration::from_secs(5)).await);
        assert_eq!(fake.written.lock().unwrap().last().unwrap(), &serde_json::json!({ "n": 21 }));
        assert_eq!(fake.keys.lock().unwrap().last().unwrap()["k"], "first again");
    }

    /// Another process, as a deploy runs one, overtakes every write of this one for more than
    /// `RENUMBER_AT_MOST` renumbers, then stops: the document is kept, not dropped, and lands.
    #[tokio::test(start_paused = true)]
    async fn a_document_overtaken_more_than_renumber_at_most_times_is_kept_and_lands_once_the_other_process_stops() {
        let overtakes = 3 * RENUMBER_AT_MOST as usize;
        let fake = Arc::new(Fake { row: Mutex::new((100, 100)), overtakes: AtomicUsize::new(overtakes), ..Fake::default() });
        let writes = writes(&fake);
        writes.queue_store(store(1), vec![]);
        settle().await;
        assert_eq!(fake.tried.load(Ordering::SeqCst), 1 + RENUMBER_AT_MOST as usize, "written once, renumbered {RENUMBER_AT_MOST} times, then a backoff");
        assert_eq!(writes.pending().store, 1, "the document is kept");
        let started = tokio::time::Instant::now();
        assert!(writes.flush(Duration::from_secs(10)).await);
        assert!(started.elapsed() < Duration::from_secs(2), "the flush hurries the backoffs: {:?}", started.elapsed());
        assert_eq!(fake.tried.load(Ordering::SeqCst), overtakes + 1);
        assert_eq!(*fake.written.lock().unwrap(), vec![serde_json::json!({ "n": 1 })], "the document ends in the row");
        assert_eq!(fake.row.lock().unwrap().0, *fake.versions.lock().unwrap().last().unwrap());
    }

    /// The process's own write that it gave up on lands, so the retry of the same number finds it
    /// in the row: the document is written again as a higher number, which changes nothing.
    #[tokio::test(start_paused = true)]
    async fn a_retry_that_finds_its_own_given_up_write_in_the_row_writes_the_same_document_higher() {
        let fake = Arc::new(Fake { lost_answers: AtomicUsize::new(1), ..Fake::default() });
        let writes = writes(&fake);
        writes.queue_store(store(1), vec![]);
        assert!(writes.flush(Duration::from_secs(5)).await);
        assert_eq!(*fake.versions.lock().unwrap(), vec![1, 2]);
        assert_eq!(*fake.written.lock().unwrap(), vec![serde_json::json!({ "n": 1 }); 2]);
        writes.queue_store(store(2), vec![]);
        assert!(writes.flush(Duration::from_secs(5)).await);
        assert_eq!(*fake.versions.lock().unwrap(), vec![1, 2, 3], "numbers only grow");
    }

    #[tokio::test(start_paused = true)]
    async fn a_document_postgres_refuses_fails_once_and_a_later_save_lands() {
        let fake = Arc::new(Fake::default());
        let writes = writes(&fake);
        writes.queue_store(Some(br#"{"n":"a\u0000b"}"#.to_vec()), vec![]);
        assert!(writes.flush(Duration::from_secs(5)).await, "the refused document leaves its slot");
        assert_eq!(fake.tried.load(Ordering::SeqCst), 1, "and is tried once, not retried");
        writes.queue_store(store(2), vec![]);
        assert!(writes.flush(Duration::from_secs(5)).await);
        assert_eq!(*fake.written.lock().unwrap(), vec![serde_json::json!({ "n": 2 })]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_flush_of_every_account_also_writes_a_save_made_after_its_account_was_flushed() {
        let (gate_a, gate_b) = (Arc::new(Semaphore::new(1)), Arc::new(Semaphore::new(0)));
        let quick = Arc::new(Fake { gate: Some(gate_a.clone()), ..Fake::default() });
        let slow = Arc::new(Fake { gate: Some(gate_b.clone()), ..Fake::default() });
        let (a, b) = (writes(&quick), writes(&slow));
        a.queue_store(store(1), vec![]);
        b.queue_store(store(1), vec![]);
        let late = {
            let (a, quick) = (a.clone(), quick.clone());
            tokio::spawn(async move {
                while quick.written.lock().unwrap().is_empty() {
                    tokio::task::yield_now().await;
                }
                // Account a's flush has returned; a connector saves again while b still writes,
                // and a's write of it takes a second.
                settle().await;
                a.queue_store(store(2), vec![]);
                settle().await;
                gate_b.add_permits(10);
                tokio::time::sleep(Duration::from_secs(1)).await;
                gate_a.add_permits(10);
            })
        };
        let accounts = || vec![(Uuid::from_u128(1), a.clone()), (Uuid::from_u128(2), b.clone())];
        let (waiting, left) = flush_all(accounts, Duration::from_secs(5)).await;
        assert_eq!(a.pending(), Pending::default(), "the save made during the flush was written before the flush ended");
        assert_eq!((waiting, left), (2, vec![]));
        late.await.unwrap();
        assert_eq!(quick.written.lock().unwrap().last().unwrap(), &serde_json::json!({ "n": 2 }));
    }
}
