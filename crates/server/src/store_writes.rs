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

/// A document as the writer hands it to the database.
pub(crate) enum Doc {
    Store(Arc<StoreBytes>),
    Secrets(BTreeMap<String, String>),
}

/// Writes one document of the account to its row as the version given. True when it was
/// written, false when the row already holds that version or a newer one.
pub(crate) type Write = Arc<dyn Fn(Uuid, u64, Doc) -> BoxFuture<'static, Result<bool, WriteFailure>> + Send + Sync>;

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
        slots.store = Some(Queued { seq, doc: Arc::new(StoreBytes { store, runs }), bytes });
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
        slots.secrets = Some(Queued { seq, doc: Arc::new(keys), bytes });
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
            // A writer waiting out a failure tries again now, and again every `HURRY_EVERY`.
            self.hurry.notify_one();
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

    /// Empties `kind`'s slot when it still holds `seq`, the document just written, or one the
    /// row holds a newer version than, or one Postgres refused.
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

    async fn run(self: Arc<Self>) {
        let mut failures = 0u32;
        loop {
            let queued = self.queued.notified();
            tokio::pin!(queued);
            queued.as_mut().enable();
            let mut failed = false;
            for kind in [Kind::Store, Kind::Secrets] {
                let Some((seq, doc)) = self.take(kind) else { continue };
                debug_assert!(seq > self.written_seq(kind), "a document older than one written");
                let what = match kind {
                    Kind::Store => "workflows",
                    Kind::Secrets => "connector keys",
                };
                match (self.write)(self.account, seq, doc).await {
                    Ok(true) => self.written(kind, seq),
                    Ok(false) => {
                        eprintln!("pinned-server: account {}'s {what} version {seq} not written: the row already holds a newer one", self.account);
                        self.written(kind, seq);
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
            failures = 0;
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
        /// Writes of a version up to this one answer that the row holds a newer one.
        stored: AtomicUsize,
        gate: Option<Arc<Semaphore>>,
        fail: AtomicUsize,
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
                    if fake.fail.load(Ordering::SeqCst) > 0 {
                        fake.fail.fetch_sub(1, Ordering::SeqCst);
                        return Err(WriteFailure::Failed("connection reset".into()));
                    }
                    if version <= fake.stored.load(Ordering::SeqCst) as u64 {
                        return Ok(false);
                    }
                    fake.versions.lock().unwrap().push(version);
                    match doc {
                        Doc::Store(bytes) => fake.written.lock().unwrap().push(joined(&bytes)),
                        Doc::Secrets(k) => fake.keys.lock().unwrap().push(k),
                    }
                    Ok(true)
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

    #[tokio::test]
    async fn a_document_the_row_holds_a_newer_version_than_counts_as_written() {
        let fake = Arc::new(Fake { stored: AtomicUsize::new(100), ..Fake::default() });
        let writes = writes(&fake);
        writes.queue_store(store(1), vec![]);
        assert!(writes.flush(Duration::from_secs(5)).await, "the slot empties");
        assert_eq!(fake.tried.load(Ordering::SeqCst), 1, "and it is not retried");
        assert!(fake.written.lock().unwrap().is_empty());
    }
}
