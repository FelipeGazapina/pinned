//! The writes of one account's row behind `PgStorage`: the store document (workflows and runs)
//! and the connector keys, as origin/main writes them: every save sends a whole document on an
//! unbounded channel, and the writer collapses a burst to the newest of each kind.
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

pub(crate) enum Doc {
    Store(Value),
    Secrets(BTreeMap<String, String>),
}

pub(crate) type Write = Arc<dyn Fn(Uuid, Doc) -> BoxFuture<'static, Result<(), String>> + Send + Sync>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub store: usize,
    pub store_bytes: usize,
    pub secrets: usize,
    pub secrets_bytes: usize,
}

enum Item {
    Store(Value, usize),
    Secrets(BTreeMap<String, String>, usize),
}

pub(crate) struct Writes {
    send: mpsc::UnboundedSender<Option<Item>>,
    pending: Mutex<Pending>,
    busy: AtomicUsize,
}

impl Writes {
    pub(crate) fn start(account: Uuid, write: Write) -> Arc<Writes> {
        let (send, mut queue) = mpsc::unbounded_channel::<Option<Item>>();
        let writes = Arc::new(Writes { send, pending: Mutex::default(), busy: AtomicUsize::new(0) });
        let me = writes.clone();
        tokio::spawn(async move {
            while let Some(Some(first)) = queue.recv().await {
                me.busy.fetch_add(1, Ordering::SeqCst);
                let (mut store, mut secrets, mut taken) = (None, None, vec![]);
                let mut next = Some(first);
                while let Some(item) = next {
                    match item {
                        Item::Store(v, n) => { taken.push((true, n)); store = Some(v) }
                        Item::Secrets(v, n) => { taken.push((false, n)); secrets = Some(v) }
                    }
                    next = queue.try_recv().ok().flatten();
                }
                if let Some(store) = store {
                    if let Err(e) = write(account, Doc::Store(store)).await {
                        eprintln!("pinned-server: could not save account {account}'s workflows: {e}");
                    }
                }
                if let Some(secrets) = secrets {
                    if let Err(e) = write(account, Doc::Secrets(secrets)).await {
                        eprintln!("pinned-server: could not save account {account}'s connector keys: {e}");
                    }
                }
                let mut pending = me.pending.lock().unwrap();
                for (store, n) in taken {
                    if store { pending.store -= 1; pending.store_bytes -= n } else { pending.secrets -= 1; pending.secrets_bytes -= n }
                }
                drop(pending);
                me.busy.fetch_sub(1, Ordering::SeqCst);
            }
        });
        writes
    }

    pub(crate) fn queue_store(&self, store: Option<Vec<u8>>, runs: Vec<u8>) -> u64 {
        let n = store.as_ref().map_or(0, Vec::len) + runs.len();
        let mut doc: Value = store.map_or(serde_json::json!({}), |s| serde_json::from_slice(&s).unwrap());
        if !runs.is_empty() {
            doc.as_object_mut().unwrap().insert("runs".into(), serde_json::from_slice(&runs).unwrap());
        }
        { let mut p = self.pending.lock().unwrap(); p.store += 1; p.store_bytes += n; }
        let _ = self.send.send(Some(Item::Store(doc, n)));
        0
    }

    pub(crate) fn queue_secrets(&self, keys: BTreeMap<String, String>) -> u64 {
        let n = keys.iter().map(|(k, v)| k.len() + v.len()).sum();
        { let mut p = self.pending.lock().unwrap(); p.secrets += 1; p.secrets_bytes += n; }
        let _ = self.send.send(Some(Item::Secrets(keys, n)));
        0
    }

    pub fn pending(&self) -> Pending {
        *self.pending.lock().unwrap()
    }

    pub async fn flush(&self, within: Duration) -> bool {
        let until = tokio::time::Instant::now() + within;
        while self.pending() != Pending::default() {
            if tokio::time::Instant::now() >= until {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        true
    }

    pub(crate) fn close(&self) {
        let _ = self.send.send(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Semaphore;

    /// A database whose writes wait for a permit each, and that can fail the next ones.
    #[derive(Default)]
    struct Fake {
        written: Mutex<Vec<Value>>,
        keys: Mutex<Vec<BTreeMap<String, String>>>,
        gate: Option<Arc<Semaphore>>,
        fail: AtomicUsize,
        tried: AtomicUsize,
    }

    fn writes(fake: &Arc<Fake>) -> Arc<Writes> {
        let fake = fake.clone();
        Writes::start(
            Uuid::nil(),
            Arc::new(move |_, doc| {
                let fake = fake.clone();
                Box::pin(async move {
                    if let Some(gate) = &fake.gate {
                        gate.acquire().await.unwrap().forget();
                    }
                    fake.tried.fetch_add(1, Ordering::SeqCst);
                    if fake.fail.load(Ordering::SeqCst) > 0 {
                        fake.fail.fetch_sub(1, Ordering::SeqCst);
                        return Err("connection reset".into());
                    }
                    match doc {
                        Doc::Store(v) => fake.written.lock().unwrap().push(v),
                        Doc::Secrets(k) => fake.keys.lock().unwrap().push(k),
                    }
                    Ok(())
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
}
