//! One account's copy for the desktops that keep it, and the handback of what they changed while
//! the hub was away. A mirror from the machine a hold names releases that hold.

use super::{Account, Hub};
use pinned_core::account_copy::{self, Author, Changes, Copy, Item, Mirrored, Progress, Taken, COPY_EVERY, MIRROR_HOLD};
use pinned_core::connectors::Connectors;
use pinned_core::taste::Tastes;
use pinned_core::wire::MachineId;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, watch, Notify};
use uuid::Uuid;

struct Sent {
    tag: String,
    items: String,
    at: Instant,
}

pub struct Copies {
    waiters: Mutex<HashMap<Uuid, Vec<(u64, oneshot::Sender<()>)>>>,
    sent: Mutex<HashMap<MachineId, Sent>>,
    stopping: watch::Sender<bool>,
    held: AtomicUsize,
    idle: Notify,
    next: AtomicU64,
}

/// Held from the moment a mirror or handback is taken until its reply is queued.
pub struct AnswerHold<'a> {
    copies: &'a Copies,
}

impl Drop for AnswerHold<'_> {
    fn drop(&mut self) {
        self.copies.held.fetch_sub(1, Ordering::AcqRel);
        self.copies.idle.notify_waiters();
    }
}

impl Copies {
    pub fn new() -> Copies {
        Copies {
            waiters: Mutex::default(),
            sent: Mutex::default(),
            stopping: watch::channel(false).0,
            held: AtomicUsize::new(0),
            idle: Notify::new(),
            next: AtomicU64::new(0),
        }
    }

    pub fn answer_hold(&self) -> AnswerHold<'_> {
        self.held.fetch_add(1, Ordering::AcqRel);
        AnswerHold { copies: self }
    }

    /// Mirrors of `account` parked inside the hold. A test waits until one is, then changes the account.
    pub fn parked(&self, account: Uuid) -> usize {
        self.waiters.lock().unwrap().get(&account).map_or(0, Vec::len)
    }

    pub fn changed(&self, account: Uuid) {
        let parked = self.waiters.lock().unwrap().remove(&account).unwrap_or_default();
        for (_, waiter) in parked {
            let _ = waiter.send(());
        }
    }

    fn enqueue(&self, account: Uuid) -> (u64, oneshot::Receiver<()>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.waiters.lock().unwrap().entry(account).or_default().push((id, sender));
        (id, receiver)
    }

    fn forget(&self, account: Uuid, id: u64) {
        let mut waiters = self.waiters.lock().unwrap();
        let Some(parked) = waiters.get_mut(&account) else { return };
        parked.retain(|(kept, _)| *kept != id);
        if parked.is_empty() {
            waiters.remove(&account);
        }
    }

    fn should_send(&self, machine: &str, have: &str, copy: &Copy) -> bool {
        let tag = copy.tag();
        let items = items_tag(copy);
        let sent = self.sent.lock().unwrap();
        let last = sent.get(machine).map(|sent| (sent.tag.clone(), sent.items.clone(), sent.at.elapsed()));
        due(have, &tag, &items, last.as_ref().map(|(tag, items, age)| (tag.as_str(), items.as_str(), *age)))
    }

    fn remember(&self, machine: &str, copy: &Copy) {
        self.sent.lock().unwrap().insert(machine.to_owned(), Sent { tag: copy.tag(), items: items_tag(copy), at: Instant::now() });
    }
}

/// Whether a mirror that has `have` should be sent `copy` now. `last` is the tag and items tag of
/// the copy last sent to that machine, and how long ago, when one was.
pub fn due(have: &str, tag: &str, items: &str, last: Option<(&str, &str, Duration)>) -> bool {
    if have == tag {
        return false;
    }
    match last {
        Some((last_tag, last_items, age)) if have == last_tag && items == last_items && tag != last_tag => age >= COPY_EVERY,
        _ => true,
    }
}

fn items_tag(copy: &Copy) -> String {
    Copy { items: copy.items.clone(), progress: Progress::default(), fixed: Value::Null, runner: None }.tag()
}

fn delivered(copies: &Copies, machine: &str, copy: &Copy, stopping: bool) -> Mirrored {
    copies.remember(machine, copy);
    Mirrored::Copy { copy: Box::new(copy.clone()), stopping }
}

pub async fn mirror(hub: &Arc<Hub>, account: &Arc<Account>, machine: &str, have: &str, login: &str, boxed: bool) -> Result<Mirrored, String> {
    admit(hub, account, machine, login, boxed).await?;
    release_if_held(hub, account, machine).await?;
    let connectors = hub.connectors(account).await?;
    if connectors.runner().is_none() {
        connectors.set_runner(Some(machine.to_owned()))?;
        hub.push_settings(account).await;
    }
    let copy = snapshot(hub, account).await?;
    if *hub.copies.stopping.borrow() {
        return Ok(delivered(&hub.copies, machine, &copy, true));
    }
    if hub.copies.should_send(machine, have, &copy) {
        return Ok(delivered(&hub.copies, machine, &copy, false));
    }
    let (id, mut receiver) = hub.copies.enqueue(account.id);
    if *hub.copies.stopping.borrow() {
        hub.copies.forget(account.id, id);
        return Ok(delivered(&hub.copies, machine, &snapshot(hub, account).await?, true));
    }
    let mut stopping = hub.copies.stopping.subscribe();
    tokio::select! {
        _ = &mut receiver => {}
        _ = tokio::time::sleep(MIRROR_HOLD) => {}
        _ = stopping.wait_for(|stopping| *stopping) => {}
    }
    hub.copies.forget(account.id, id);
    let copy = snapshot(hub, account).await?;
    if *hub.copies.stopping.borrow() {
        return Ok(delivered(&hub.copies, machine, &copy, true));
    }
    if hub.copies.should_send(machine, have, &copy) {
        return Ok(delivered(&hub.copies, machine, &copy, false));
    }
    Ok(Mirrored::Unchanged { stopping: false })
}

pub async fn handback(hub: &Arc<Hub>, account: &Arc<Account>, machine: &str, changes: &Changes, login: &str, boxed: bool) -> Result<Taken, String> {
    let name = admit(hub, account, machine, login, boxed).await?;
    let connectors = hub.connectors(account).await?;
    if !changes.id.is_nil() && connectors.applied_handback(machine) == Some(changes.id) {
        return Ok(Taken { notes: Vec::new(), copy: snapshot(hub, account).await? });
    }
    let author = Author { id: machine.to_owned(), name };
    let mut recorded = hub.db.offline_edits(account.id).await?;
    let mut notes = connectors.take_back(changes, &mut recorded, &author)?;
    let taste_edits: BTreeMap<Item, _> = changes.edits.iter().filter(|(item, _)| item.is_taste()).map(|(item, edit)| (item.clone(), edit.clone())).collect();
    if !taste_edits.is_empty() {
        let stored = hub.db.tastes(account.id).await?.unwrap_or_default();
        let mut current = stored.copy_items();
        let norm = |item: &Item, version: account_copy::Version| Tastes::normalize_copy_item(item, &version);
        notes.extend(account_copy::merge(&mut current, &mut recorded, &taste_edits, &author, &norm));
        hub.db.save_tastes(account.id, &stored.with_copy_items(&current)?).await?;
    }
    hub.db.keep_offline_edits(account.id, &recorded).await?;
    release_if_held(hub, account, machine).await?;
    if !changes.id.is_nil() {
        connectors.remember_handback(machine.to_owned(), changes.id)?;
    }
    hub.push_settings(account).await;
    Ok(Taken { notes, copy: snapshot(hub, account).await? })
}

async fn release_if_held(hub: &Arc<Hub>, account: &Arc<Account>, machine: &str) -> Result<(), String> {
    let connectors = hub.connectors(account).await?;
    if connectors.held_for().as_deref() != Some(machine) {
        return Ok(());
    }
    connectors.release()?;
    connectors.wake();
    hub.watch(account).await?;
    hub.push_settings(account).await;
    Ok(())
}

async fn admit(hub: &Hub, account: &Account, machine: &str, login: &str, boxed: bool) -> Result<String, String> {
    if account.team {
        return Err("a team's account keeps no copy".into());
    }
    if boxed || account.dedicated(machine) {
        return Err("a box may not keep a copy of the account's keys".into());
    }
    if login.is_empty() {
        return Err("that login is not the account's".into());
    }
    let opened = hub.db.login(login, false).await?.ok_or_else(|| "that login is not the account's".to_owned())?;
    if opened.account != account.id {
        return Err("that login is not the account's".into());
    }
    let id = Uuid::parse_str(machine).map_err(|_| format!("this account has no machine {machine}"))?;
    hub.db.machine_name(account.id, id).await?.ok_or_else(|| format!("this account has no machine {machine}"))
}

async fn snapshot(hub: &Hub, account: &Account) -> Result<Copy, String> {
    let connectors = hub.connectors(account).await?;
    let (mut items, progress, fixed) = connectors.copy_parts();
    if let Some(tastes) = hub.db.tastes(account.id).await? {
        items.extend(tastes.copy_items());
    }
    Ok(Copy { items, progress, fixed, runner: connectors.runner() })
}

/// The accounts whose runner is set, and whose last beat is old enough, wait for that runner.
/// No beat row holds nothing: a hub that just started has not been away.
pub async fn hold_after_absence(hub: &Arc<Hub>) -> Result<(), String> {
    let Some(age) = hub.db.beat_age().await? else { return Ok(()) };
    if age < account_copy::HOLD_AFTER {
        return Ok(());
    }
    for id in hub.db.accounts_with_connectors().await? {
        let account = hub.account(id).await?;
        if account.team {
            continue;
        }
        hub.connectors(&account).await?.hold_for_runner()?;
    }
    Ok(())
}

/// Stops finding, then answers every mirror still held with `stopping: true`, and waits until
/// those replies are queued.
pub async fn stop(hub: &Hub) {
    let connectors: Vec<Arc<Connectors>> = hub.accounts.lock().unwrap().values().filter_map(|account| account.connectors.get().cloned()).collect();
    for connectors in &connectors {
        connectors.stop_finding();
    }
    let started = Instant::now();
    while connectors.iter().any(|connectors| connectors.checks_open() > 0) && started.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    hub.copies.stopping.send_replace(true);
    let parked = std::mem::take(&mut *hub.copies.waiters.lock().unwrap());
    for (_, parked) in parked {
        for (_, waiter) in parked {
            let _ = waiter.send(());
        }
    }
    let started = Instant::now();
    while hub.copies.held.load(Ordering::Acquire) > 0 && started.elapsed() < Duration::from_secs(5) {
        let notify = hub.copies.idle.notified();
        if hub.copies.held.load(Ordering::Acquire) == 0 {
            break;
        }
        tokio::select! {
            _ = notify => {}
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
}
