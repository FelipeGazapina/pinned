//! The account's own-machine state as a machine keeps a copy of it, and how what changed on the
//! copy while the hub was away goes back. Pure: no files, no sockets, no clocks.

use crate::connectors::{Filed, Followed, Paged};
use crate::wire::MachineId;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

/// How long the hub's address must answer "no hub here" before the offline runner takes over.
pub const TAKEOVER_AFTER: Duration = Duration::from_secs(5 * 60);
/// A hub that finds its last beat older than this holds each account's workflows for its runner.
pub const HOLD_AFTER: Duration = Duration::from_secs(2 * 60);
const _: () = assert!(HOLD_AFTER.as_secs() * 2 <= TAKEOVER_AFTER.as_secs());
/// How long the hub keeps a `Mirror` call before answering `Unchanged`, inside `link::CALL_WAIT`.
pub const MIRROR_HOLD: Duration = Duration::from_secs(50);
const _: () = assert!(MIRROR_HOLD.as_secs() < 60);
/// How often a copy goes out for progress alone. A change of items, or a stop, sends one at once.
pub const COPY_EVERY: Duration = Duration::from_secs(5 * 60);
/// A copy heard from the hub this long before it went counts as current at the takeover.
pub const FRESH: Duration = Duration::from_secs(90);

pub(crate) const TASTE_FIELDS: &[&str] = &[
    "strategy",
    "ladder",
    "hard_model",
    "parallel",
    "autonomy",
    "review_diffs",
    "second_opinion",
    "skills",
    "context",
    "tier",
    "default_effort",
    "max_effort",
    "budget_cents",
    "time_limit_min",
    "box_budget_cents",
    "box_size",
];

/// A thing of the account a person edits as a whole, named the same on every machine.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
pub struct Item(String);

/// What a version that lost to another becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loser {
    /// Kept beside the winner, turned off, named for where it came from (workflows, MCP servers).
    SetAside,
    /// Dropped and named in a note (integrations, Convex, taste fields).
    Noted,
}

impl Item {
    pub fn workflow(id: &str) -> Item {
        Item(format!("workflow/{id}"))
    }
    pub fn mcp_server(id: &str) -> Item {
        Item(format!("mcp/{id}"))
    }
    pub fn slack_bot(id: &str) -> Item {
        Item(format!("slack-bot/{id}"))
    }
    pub fn convex(id: &str) -> Item {
        Item(format!("convex/{id}"))
    }
    pub fn integration(name: &str) -> Item {
        Item(name.to_owned())
    }
    pub fn taste(path: &str) -> Item {
        Item(format!("taste/{path}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn is_taste(&self) -> bool {
        self.0.starts_with("taste/")
    }
    pub fn loser(&self) -> Loser {
        if self.0.starts_with("workflow/") || self.0.starts_with("mcp/") { Loser::SetAside } else { Loser::Noted }
    }
}

impl fmt::Debug for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Item {
    type Error = String;

    fn try_from(text: String) -> Result<Item, String> {
        let id_ok = |id: &str| !id.is_empty() && !id.contains('/') && id != "." && id != "..";
        let prefixed = |prefix: &str| text.strip_prefix(prefix).filter(|id| id_ok(id));
        if prefixed("workflow/").is_some() || prefixed("mcp/").is_some() || prefixed("slack-bot/").is_some() || prefixed("convex/").is_some() {
            return Ok(Item(text));
        }
        if matches!(text.as_str(), "linear" | "github" | "vercel" | "taste/placement" | "taste/reviewer" | "taste/worker-profiles") {
            return Ok(Item(text));
        }
        if let Some(repo) = text.strip_prefix("taste/project/") {
            if !repo.is_empty() {
                return Ok(Item(text));
            }
        }
        if let Some(field) = text.strip_prefix("taste/orchestrator/").or_else(|| text.strip_prefix("taste/captains/")) {
            if TASTE_FIELDS.contains(&field) {
                return Ok(Item(text));
            }
        }
        Err(format!("unknown account item {text:?}"))
    }
}

impl From<Item> for String {
    fn from(item: Item) -> String {
        item.0
    }
}

/// An item as one machine has it. Secrets are keyed by their role (`key`, `env:<NAME>`, `header:<NAME>`).
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Version {
    pub value: Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, String>,
}

impl fmt::Debug for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Version").field("value", &self.value).field("secrets", &redacted(&self.secrets)).finish()
    }
}

/// The last four characters of a secret, for a note or a log line.
pub fn secret_tail(secret: &str) -> String {
    let last: String = secret.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("…{last}")
}

fn redacted(secrets: &BTreeMap<String, String>) -> BTreeMap<&str, String> {
    secrets.iter().map(|(role, value)| (role.as_str(), secret_tail(value))).collect()
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), canonical(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// SHA-256 hex of the normalized version's canonical JSON, secrets included.
pub fn version_hash(version: &Version) -> String {
    let value = canonical(&serde_json::to_value(version).expect("a version is json"));
    let bytes = serde_json::to_vec(&value).expect("canonical json");
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

/// What the account's watchers did. Each part only grows, so [`Progress::join`] takes any two in any order.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Progress {
    #[serde(default)]
    pub workflows: BTreeMap<String, WorkflowProgress>,
    /// The bot that claimed each Slack thread, by `<channel>:<ts>`. The first claim stays.
    #[serde(default)]
    pub slack_threads: BTreeMap<String, String>,
    #[serde(default)]
    pub fires: BTreeSet<Fire>,
    #[serde(default)]
    pub change_retries: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub apps_seen: BTreeMap<String, String>,
    /// None when the two sides disagree, so the next check reads from the watermark again.
    #[serde(default)]
    pub linear_changes_from: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct WorkflowProgress {
    pub since: String,
    pub handled: BTreeSet<String>,
    pub created: BTreeMap<String, Filed>,
    pub followed: BTreeMap<String, Followed>,
    pub paged: BTreeMap<String, Paged>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Fire {
    pub workflow: String,
    pub key: String,
    pub at: i64,
}

impl Progress {
    pub fn join(&mut self, other: &Progress) {
        for (id, theirs) in &other.workflows {
            self.workflows.entry(id.clone()).and_modify(|ours| ours.join(theirs)).or_insert_with(|| theirs.clone());
        }
        for (thread, bot) in &other.slack_threads {
            self.slack_threads.entry(thread.clone()).and_modify(|ours| {
                if bot.as_str() < ours.as_str() {
                    *ours = bot.clone();
                }
            }).or_insert_with(|| bot.clone());
        }
        self.fires.extend(other.fires.iter().cloned());
        for (workflow, keys) in &other.change_retries {
            let slot = self.change_retries.entry(workflow.clone()).or_default();
            for (key, at) in keys {
                slot.entry(key.clone()).and_modify(|ours| {
                    if at.as_str() > ours.as_str() {
                        *ours = at.clone();
                    }
                }).or_insert_with(|| at.clone());
            }
        }
        for (workflow, seen) in &other.apps_seen {
            self.apps_seen.entry(workflow.clone()).and_modify(|ours| {
                if seen.as_str() > ours.as_str() {
                    *ours = seen.clone();
                }
            }).or_insert_with(|| seen.clone());
        }
        self.linear_changes_from = match (&self.linear_changes_from, &other.linear_changes_from) {
            (Some(a), Some(b)) if a == b => Some(a.clone()),
            _ => None,
        };
    }
}

impl WorkflowProgress {
    fn join(&mut self, other: &WorkflowProgress) {
        self.since = later_since(&self.since, &other.since);
        self.handled.extend(other.handled.iter().cloned());
        for (key, filed) in &other.created {
            self.created.entry(key.clone()).and_modify(|ours| *ours = earlier_filed(ours, filed)).or_insert_with(|| filed.clone());
        }
        for (key, followed) in &other.followed {
            self.followed.entry(key.clone()).and_modify(|ours| *ours = further_followed(ours, followed)).or_insert_with(|| followed.clone());
        }
        for (key, paged) in &other.paged {
            self.paged.entry(key.clone()).and_modify(|ours| *ours = later_paged(ours, paged)).or_insert_with(|| paged.clone());
        }
    }
}

fn later_since(a: &str, b: &str) -> String {
    if a.is_empty() {
        return b.to_owned();
    }
    if b.is_empty() {
        return a.to_owned();
    }
    let order = match (chrono::DateTime::parse_from_rfc3339(a).ok(), chrono::DateTime::parse_from_rfc3339(b).ok()) {
        (Some(left), Some(right)) => left.cmp(&right).then_with(|| a.cmp(b)),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => a.cmp(b),
    };
    if order == std::cmp::Ordering::Less { b.to_owned() } else { a.to_owned() }
}

fn earlier_filed(a: &Filed, b: &Filed) -> Filed {
    let left = (a.id.as_str(), a.identifier.as_str(), a.url.as_str());
    let right = (b.id.as_str(), b.identifier.as_str(), b.url.as_str());
    if left <= right { a.clone() } else { b.clone() }
}

fn further_followed(a: &Followed, b: &Followed) -> Followed {
    let further = if a.cursor >= b.cursor { a } else { b };
    Followed { cursor: further.cursor.clone(), seen: later_seen(a.seen.as_deref(), b.seen.as_deref()), replied: a.replied || b.replied }
}

fn later_seen(a: Option<&str>, b: Option<&str>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a >= b { a } else { b }.to_owned()),
        (Some(a), None) => Some(a.to_owned()),
        (None, Some(b)) => Some(b.to_owned()),
        (None, None) => None,
    }
}

fn later_paged(a: &Paged, b: &Paged) -> Paged {
    if b.at > a.at {
        b.clone()
    } else if a.at > b.at {
        a.clone()
    } else {
        Paged { at: a.at.clone(), ping: a.ping || b.ping }
    }
}

/// The account as a machine keeps it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Copy {
    pub items: BTreeMap<Item, Version>,
    pub progress: Progress,
    pub fixed: Value,
    #[serde(default)]
    pub runner: Option<MachineId>,
}

impl Copy {
    /// The first 16 hex digits of the SHA-256 of the canonical JSON.
    pub fn tag(&self) -> String {
        let value = canonical(&serde_json::to_value(self).expect("a copy is json"));
        let bytes = serde_json::to_vec(&value).expect("canonical json");
        hex(&Sha256::digest(bytes))[..16].to_owned()
    }
}

/// The hub's answer to `HubCall::Mirror`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mirrored {
    Copy { copy: Box<Copy>, stopping: bool },
    Unchanged { stopping: bool },
}

/// One item this machine changed on its copy.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Edit {
    pub at: u64,
    pub base: Option<Version>,
    pub ours: Option<Version>,
}

/// A `pinned run set` for a run the hub owns, kept until the hub can take it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct RunSet {
    pub connector: String,
    #[serde(flatten)]
    pub subject: crate::agent::Subject,
    pub name: String,
    pub value: String,
}

fn nil_id() -> uuid::Uuid {
    uuid::Uuid::nil()
}

/// What a machine hands back when it reaches the hub again.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Changes {
    #[serde(default = "nil_id")]
    #[schemars(with = "String")]
    pub id: uuid::Uuid,
    #[serde(default)]
    pub edits: BTreeMap<Item, Edit>,
    #[serde(default)]
    pub progress: Progress,
    #[serde(default)]
    pub runs: Vec<Value>,
    #[serde(default)]
    pub run_sets: Vec<RunSet>,
    #[serde(default)]
    pub took_over_at: Option<u64>,
}

impl Default for Changes {
    fn default() -> Self {
        Changes { id: uuid::Uuid::nil(), edits: BTreeMap::new(), progress: Progress::default(), runs: Vec::new(), run_sets: Vec::new(), took_over_at: None }
    }
}

/// The hub's answer to `HubCall::Handback`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Taken {
    pub notes: Vec<Note>,
    pub copy: Copy,
}

/// A sentence about an item the merge did not take as it came.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Note {
    pub item: Item,
    pub text: String,
}

/// Which machine wrote the version the hub keeps, named by the hash of that version. The version
/// itself stays in the sealed account, so this record holds no key.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Recorded {
    pub hash: String,
    pub at: u64,
    pub machine: MachineId,
    pub machine_name: String,
}

/// The machine a handback comes from, as the hub knows it.
pub struct Author {
    pub id: MachineId,
    pub name: String,
}

/// Folds one machine's edits into the account's items. `normalize` puts a version in this code's
/// own form. Updates `recorded` for each version taken from `edits`.
pub fn merge(
    current: &mut BTreeMap<Item, Version>,
    recorded: &mut BTreeMap<Item, Recorded>,
    edits: &BTreeMap<Item, Edit>,
    from: &Author,
    normalize: &dyn Fn(&Item, Version) -> Version,
) -> Vec<Note> {
    let mut notes = Vec::new();
    for (item, edit) in edits {
        let base = edit.base.clone().map(|version| normalize(item, version));
        let ours = edit.ours.clone().map(|version| normalize(item, version));
        let cur = current.get(item).cloned().map(|version| normalize(item, version));
        if cur == ours {
            remember(recorded, item, ours.as_ref(), edit.at, from);
            continue;
        }
        if cur == base {
            put(current, recorded, item, ours, edit.at, from);
            continue;
        }
        match (&cur, &ours) {
            (None, Some(ours)) => {
                put(current, recorded, item, Some(ours.clone()), edit.at, from);
                notes.push(Note { item: item.clone(), text: format!("deleted elsewhere, kept {}'s change", from.name) });
            }
            (Some(_), None) => {
                notes.push(Note { item: item.clone(), text: format!("{} deleted it, but it changed elsewhere", from.name) });
            }
            (Some(cur), Some(ours)) => {
                let stamp = recorded.get(item).filter(|stamp| stamp.hash == version_hash(cur)).cloned();
                let ours_wins = stamp.as_ref().is_some_and(|stamp| (edit.at, from.id.as_str()) > (stamp.at, stamp.machine.as_str()));
                let (winner, loser, loser_from) = if ours_wins {
                    (Some(ours.clone()), cur, stamp.as_ref().map(|stamp| stamp.machine_name.as_str()).unwrap_or("the hub"))
                } else {
                    (None, ours, from.name.as_str())
                };
                match item.loser() {
                    Loser::SetAside => {
                        let (aside, version) = set_aside(item, loser, loser_from);
                        current.entry(aside).or_insert(version);
                    }
                    Loser::Noted => notes.push(Note { item: item.clone(), text: noted(loser_from, loser) }),
                }
                if let Some(winner) = winner {
                    put(current, recorded, item, Some(winner), edit.at, from);
                }
            }
            (None, None) => {}
        }
    }
    notes
}

fn remember(recorded: &mut BTreeMap<Item, Recorded>, item: &Item, version: Option<&Version>, at: u64, from: &Author) {
    match version {
        Some(version) => {
            recorded.insert(item.clone(), stamp(version, at, from));
        }
        None => {
            recorded.remove(item);
        }
    }
}

fn put(current: &mut BTreeMap<Item, Version>, recorded: &mut BTreeMap<Item, Recorded>, item: &Item, version: Option<Version>, at: u64, from: &Author) {
    match version {
        Some(version) => {
            recorded.insert(item.clone(), stamp(&version, at, from));
            current.insert(item.clone(), version);
        }
        None => {
            recorded.remove(item);
            current.remove(item);
        }
    }
}

fn stamp(version: &Version, at: u64, from: &Author) -> Recorded {
    Recorded { hash: version_hash(version), at, machine: from.id.clone(), machine_name: from.name.clone() }
}

fn noted(from: &str, loser: &Version) -> String {
    let keys = loser.secrets.iter().map(|(role, value)| format!("{role} {}", secret_tail(value))).collect::<Vec<_>>().join(", ");
    if keys.is_empty() { format!("{from} changed it while the hub was off; that change was not kept") } else { format!("{from} changed it while the hub was off; that change was not kept ({keys})") }
}

/// The turned-off copy of a losing workflow or MCP server.
pub fn set_aside(item: &Item, loser: &Version, from: &str) -> (Item, Version) {
    let hash8 = &version_hash(loser)[..8];
    let mut value = loser.value.clone();
    let (id, name) = match value.as_object() {
        Some(object) => {
            let id = object.get("id").and_then(Value::as_str).unwrap_or("item").to_owned();
            let name = object.get("name").and_then(Value::as_str).or_else(|| object.get("server").and_then(|server| server.get("name")).and_then(Value::as_str)).unwrap_or("item").to_owned();
            (id, name)
        }
        None => ("item".into(), "item".into()),
    };
    let new_id = format!("{id}-{hash8}");
    let new_name = format!("{name} ({from} while the hub was off)");
    if let Some(object) = value.as_object_mut() {
        object.insert("id".into(), Value::String(new_id.clone()));
        object.insert("enabled".into(), Value::Bool(false));
        if object.contains_key("name") {
            object.insert("name".into(), Value::String(new_name));
        } else if let Some(server) = object.get_mut("server").and_then(Value::as_object_mut) {
            server.insert("name".into(), Value::String(new_name));
        }
    }
    let aside = if item.as_str().starts_with("mcp/") { Item::mcp_server(&new_id) } else { Item::workflow(&new_id) };
    (aside, Version { value, secrets: loser.secrets.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::refuse_real_home;
    use serde_json::json;

    fn version(value: Value) -> Version {
        Version { value, secrets: BTreeMap::new() }
    }

    fn secret_version(value: Value, secret: &str) -> Version {
        Version { value, secrets: BTreeMap::from([("key".into(), secret.into())]) }
    }

    fn identity(_: &Item, version: Version) -> Version {
        version
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() as usize) % n.max(1)
        }
    }

    fn filed(rng: &mut Rng) -> Filed {
        let n = rng.below(4);
        Filed { id: format!("id{n}"), identifier: format!("T-{n}"), url: format!("https://linear.example/{n}") }
    }

    fn followed(rng: &mut Rng) -> Followed {
        Followed { cursor: format!("170000000{}.00010{}", rng.below(4), rng.below(3)), seen: (rng.below(2) == 0).then(|| format!("170000000{}.000200", rng.below(3))), replied: rng.below(2) == 0 }
    }

    fn paged(rng: &mut Rng) -> Paged {
        Paged { at: format!("2026-11-19T00:0{}:00.000Z", rng.below(6)), ping: rng.below(2) == 0 }
    }

    fn gen_progress(rng: &mut Rng) -> Progress {
        let mut progress = Progress::default();
        for i in 0..rng.below(3) {
            let id = format!("w{i}");
            let mut workflow = WorkflowProgress::default();
            workflow.since = if rng.below(3) == 0 { String::new() } else { format!("2026-11-19T00:0{}:00Z", rng.below(6)) };
            for k in 0..rng.below(3) {
                workflow.handled.insert(format!("h{k}-{}", rng.below(2)));
            }
            if rng.below(2) == 0 {
                workflow.created.insert(format!("C:{i}"), filed(rng));
            }
            if rng.below(2) == 0 {
                workflow.followed.insert(format!("C:{i}"), followed(rng));
            }
            if rng.below(2) == 0 {
                workflow.paged.insert(format!("C:{i}"), paged(rng));
            }
            progress.workflows.insert(id, workflow);
        }
        if rng.below(2) == 0 {
            progress.slack_threads.insert(format!("C{}:1.0", rng.below(3)), format!("bot-{}", rng.below(3)));
        }
        if rng.below(2) == 0 {
            progress.fires.insert(Fire { workflow: format!("w{}", rng.below(2)), key: format!("k{}", rng.below(3)), at: rng.below(5) as i64 });
        }
        if rng.below(2) == 0 {
            progress.change_retries.entry(format!("w{}", rng.below(2))).or_default().insert(format!("k{}", rng.below(2)), format!("2026-11-19T00:0{}:00.000Z", rng.below(4)));
        }
        if rng.below(2) == 0 {
            progress.apps_seen.insert(format!("w{}", rng.below(2)), format!("C{}:170000000{}.0", rng.below(2), rng.below(4)));
        }
        progress.linear_changes_from = (rng.below(3) != 0).then(|| format!("2026-11-19T00:0{}:00Z", rng.below(3)));
        progress
    }

    fn joined(left: &Progress, right: &Progress) -> Progress {
        let mut out = left.clone();
        out.join(right);
        out
    }

    #[test]
    fn progress_join_is_commutative_associative_and_idempotent() {
        refuse_real_home();
        let mut rng = Rng(0xA11CE);
        for _ in 0..40 {
            let a = gen_progress(&mut rng);
            let b = gen_progress(&mut rng);
            let c = gen_progress(&mut rng);
            assert_eq!(joined(&a, &b), joined(&b, &a));
            assert_eq!(joined(&joined(&a, &b), &c), joined(&a, &joined(&b, &c)));
            assert_eq!(joined(&a, &a), a);
        }
    }

    fn workflow(id: &str, name: &str, instructions: &str) -> Version {
        version(json!({"id": id, "name": name, "enabled": true, "instructions": instructions}))
    }

    fn author(id: &str, name: &str) -> Author {
        Author { id: id.into(), name: name.into() }
    }

    fn edit(at: u64, base: Option<Version>, ours: Option<Version>) -> Edit {
        Edit { at, base, ours }
    }

    fn apply(current: &BTreeMap<Item, Version>, recorded: &BTreeMap<Item, Recorded>, edits: &BTreeMap<Item, Edit>, from: &Author) -> (BTreeMap<Item, Version>, BTreeMap<Item, Recorded>) {
        let mut current = current.clone();
        let mut recorded = recorded.clone();
        merge(&mut current, &mut recorded, edits, from, &identity);
        (current, recorded)
    }

    #[test]
    fn merge_order_does_not_change_the_account() {
        refuse_real_home();
        let mut rng = Rng(0xBEE5);
        for _ in 0..30 {
            let mut base_items = BTreeMap::new();
            let mut current = BTreeMap::new();
            let recorded = BTreeMap::new();
            for n in 0..3 {
                let item = Item::workflow(&format!("w{n}"));
                let base = workflow(&format!("w{n}"), "Triage", "");
                base_items.insert(item.clone(), base.clone());
                if rng.below(4) == 0 {
                    let hub = workflow(&format!("w{n}"), "Triage", "hub");
                    current.insert(item, hub);
                } else {
                    current.insert(item, base);
                }
            }
            let mut edits_a = BTreeMap::new();
            let mut edits_b = BTreeMap::new();
            for (item, base) in &base_items {
                if rng.below(2) == 0 {
                    let ours = if rng.below(5) == 0 { None } else { Some(workflow(item.as_str().trim_start_matches("workflow/"), "Triage", "from a")) };
                    edits_a.insert(item.clone(), edit(10 + rng.below(3) as u64, Some(base.clone()), ours));
                }
                if rng.below(2) == 0 {
                    let ours = if rng.below(5) == 0 { None } else { Some(workflow(item.as_str().trim_start_matches("workflow/"), "Triage", "from b")) };
                    edits_b.insert(item.clone(), edit(20 + rng.below(3) as u64, Some(base.clone()), ours));
                }
            }
            let a = author("machine-a", "MacBook");
            let b = author("machine-b", "Air");
            let (ab_items, ab_recorded) = {
                let (mid_items, mid_recorded) = apply(&current, &recorded, &edits_a, &a);
                apply(&mid_items, &mid_recorded, &edits_b, &b)
            };
            let (ba_items, ba_recorded) = {
                let (mid_items, mid_recorded) = apply(&current, &recorded, &edits_b, &b);
                apply(&mid_items, &mid_recorded, &edits_a, &a)
            };
            assert_eq!(ab_items, ba_items);
            assert_eq!(ab_recorded, ba_recorded);
            let _ = recorded;
        }
    }

    #[test]
    fn merge_twice_matches_once_and_makes_one_set_aside() {
        refuse_real_home();
        let item = Item::workflow("w");
        let base = workflow("w", "Triage", "");
        let ours = workflow("w", "Triage", "offline");
        let mut current = BTreeMap::from([(item.clone(), workflow("w", "Triage", "hub"))]);
        let mut recorded = BTreeMap::new();
        let edits = BTreeMap::from([(item.clone(), edit(10, Some(base), Some(ours)))]);
        let from = author("machine-a", "MacBook");
        merge(&mut current, &mut recorded, &edits, &from, &identity);
        let once_items = current.clone();
        let once_recorded = recorded.clone();
        let asides: Vec<_> = current.keys().filter(|key| key.as_str() != "workflow/w").cloned().collect();
        assert_eq!(asides.len(), 1);
        merge(&mut current, &mut recorded, &edits, &from, &identity);
        assert_eq!(current, once_items);
        assert_eq!(recorded, once_recorded);
    }

    #[test]
    fn the_later_desktop_keeps_the_workflow_and_the_earlier_is_set_aside() {
        refuse_real_home();
        let item = Item::workflow("w");
        let base = workflow("w", "Triage", "");
        let mac = workflow("w", "Triage", "macbook");
        let air = workflow("w", "Triage", "air");
        let edits = |at, ours: Version| BTreeMap::from([(item.clone(), edit(at, Some(base.clone()), Some(ours)))]);
        let mut left = BTreeMap::from([(item.clone(), base.clone())]);
        let mut right = left.clone();
        let mut left_recorded = BTreeMap::new();
        let mut right_recorded = BTreeMap::new();
        merge(&mut left, &mut left_recorded, &edits(10, mac.clone()), &author("a", "MacBook"), &identity);
        merge(&mut left, &mut left_recorded, &edits(12, air.clone()), &author("b", "Air"), &identity);
        merge(&mut right, &mut right_recorded, &edits(12, air.clone()), &author("b", "Air"), &identity);
        merge(&mut right, &mut right_recorded, &edits(10, mac.clone()), &author("a", "MacBook"), &identity);
        assert_eq!(left, right);
        assert_eq!(left.get(&item).unwrap(), &air);
        let aside = left.iter().find(|(key, _)| *key != &item).unwrap().1;
        assert_eq!(aside.value["name"], json!("Triage (MacBook while the hub was off)"));
        assert_eq!(aside.value["enabled"], json!(false));
        assert!(left_recorded.get(&item).unwrap().hash == version_hash(&air));
        assert!(!serde_json::to_string(&left_recorded).unwrap().contains("macbook"));
    }

    #[test]
    fn every_offline_edit_is_kept_set_aside_or_named() {
        refuse_real_home();
        let linear = Item::integration("linear");
        let workflow_item = Item::workflow("w");
        let base_w = workflow("w", "Triage", "");
        let base_linear = secret_version(json!({"team": "T"}), "lin_api_super-secret-linear-key-XYZ9");
        let mut current = BTreeMap::from([(workflow_item.clone(), base_w.clone()), (linear.clone(), base_linear.clone())]);
        let mut recorded = BTreeMap::new();
        let a_w = workflow("w", "Triage", "a");
        let b_w = workflow("w", "Triage", "b");
        let a_linear = secret_version(json!({"team": "A"}), "lin_api_other-secret-aaaa");
        let edits_a = BTreeMap::from([(workflow_item.clone(), edit(10, Some(base_w.clone()), Some(a_w.clone()))), (linear.clone(), edit(10, Some(base_linear.clone()), Some(a_linear.clone())))]);
        let edits_b = BTreeMap::from([(workflow_item.clone(), edit(12, Some(base_w), Some(b_w.clone())))]);
        let notes_a = merge(&mut current, &mut recorded, &edits_a, &author("a", "MacBook"), &identity);
        let notes_b = merge(&mut current, &mut recorded, &edits_b, &author("b", "Air"), &identity);
        assert_eq!(current.get(&workflow_item).unwrap(), &b_w);
        assert!(current.keys().any(|key| key.as_str().starts_with("workflow/w-")));
        assert_eq!(current.get(&linear).unwrap(), &a_linear);
        let notes = notes_a.into_iter().chain(notes_b).collect::<Vec<_>>();
        assert!(notes.iter().all(|note| !note.text.contains("lin_api_")));
        assert!(notes.iter().any(|note| note.text.contains("…aaaa")) || current.get(&linear).unwrap().secrets["key"].ends_with("aaaa"));
        let _ = notes;
    }

    #[test]
    fn a_workflow_missing_a_defaulted_field_merges_as_unchanged() {
        refuse_real_home();
        let old = json!({
            "id": "triage",
            "name": "Triage",
            "enabled": true,
            "trigger": {"kind": "manual"},
            "folder": "/work",
            "model": "",
            "effort": "medium",
            "permissionMode": "acceptEdits",
            "worktree": false,
            "instructions": ""
        });
        let parsed: crate::connectors::Connector = serde_json::from_value(old.clone()).expect("previous release workflow");
        let round = serde_json::to_value(&parsed).unwrap();
        assert_ne!(old, round);
        let normalize = |item: &Item, version: Version| -> Version {
            if !item.as_str().starts_with("workflow/") {
                return version;
            }
            match serde_json::from_value::<crate::connectors::Connector>(version.value.clone()) {
                Ok(connector) => Version { value: serde_json::to_value(connector).unwrap(), secrets: version.secrets },
                Err(_) => version,
            }
        };
        let item = Item::workflow("triage");
        let mut current = BTreeMap::from([(item.clone(), Version { value: round.clone(), secrets: BTreeMap::new() })]);
        let mut recorded = BTreeMap::new();
        let edits = BTreeMap::from([(item, edit(10, Some(version(old.clone())), Some(version(old))))]);
        let notes = merge(&mut current, &mut recorded, &edits, &author("m", "Mac"), &normalize);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(current.values().next().unwrap().value, round);
    }

    #[test]
    fn a_version_debug_and_its_stamp_leave_the_secret_out() {
        refuse_real_home();
        let version = secret_version(json!({"team": "T"}), "lin_api_super-secret-linear-key-XYZ9");
        let shown = format!("{version:?}");
        assert!(!shown.contains("lin_api_super-secret-linear-key-XYZ9"));
        assert!(shown.contains("…XYZ9"));
        let recorded = Recorded { hash: version_hash(&version), at: 1, machine: "m".into(), machine_name: "Mac".into() };
        let stored = serde_json::to_string(&recorded).unwrap();
        assert!(!stored.contains("lin_api_"));
        assert!(stored.contains("hash"));
        let other = secret_version(json!({"team": "T"}), "different-secret");
        assert_ne!(version_hash(&version), version_hash(&other));
    }

    #[test]
    fn an_unknown_item_name_is_refused() {
        refuse_real_home();
        assert!(Item::try_from(String::from("workflow/")).is_err());
        assert!(Item::try_from(String::from("taste/orchestrator/nope")).is_err());
        assert!(Item::try_from(String::from("taste/project/github.com/acme/site")).is_ok());
        assert!(Item::try_from(String::from("taste/worker-profiles")).is_ok());
    }
}
