//! A connectors store as a copy's items, progress and fixed part, and the store those make again.

use super::{
    mcp_key, named_claude_key, slack_bot_key, BoxAccess, BoxAsk, BoxGrant, ChangeCounts, Connector, Connectors, GITHUB_KEY, LINEAR_KEY, RepoInsight, SLACK_KEY, Storage, UPSTASH_KEY,
    VERCEL_KEY,
};
use crate::account_copy::{Author, Changes, Copy, Edit, Fire, Item, Note, Progress, Recorded, Version, WorkflowProgress};
use crate::graph::runner;
use crate::wire::MachineId;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn held_error(machine: &str) -> String {
    format!("workflows wait for {machine} to hand back what it ran while the hub was off")
}

#[derive(Serialize, serde::Deserialize, Default)]
struct Fixed {
    #[serde(default)]
    upstash: Option<crate::upstash::Account>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    box_claude_tokens: BTreeMap<String, String>,
    #[serde(default)]
    boxes: BoxAccess,
    #[serde(default)]
    box_asks: Vec<BoxAsk>,
    #[serde(default)]
    box_grants: Vec<BoxGrant>,
    #[serde(default, skip_serializing_if = "repo_off")]
    repo_insight: RepoInsight,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    change_counts: BTreeMap<String, ChangeCounts>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    box_released_at: BTreeMap<String, BTreeMap<String, u64>>,
}

fn repo_off(insight: &RepoInsight) -> bool {
    insight == &RepoInsight::default()
}

impl Connectors {
    pub fn copy_parts(&self) -> (BTreeMap<Item, Version>, Progress, Value) {
        let store = self.store.lock().unwrap();
        project(&store, self.storage.as_ref())
    }

    pub fn take_copy(&self, copy: &Copy) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        let mut names = item_key_names(&store);
        names.insert(UPSTASH_KEY.to_owned());
        for name in store.box_claude_tokens.keys() {
            names.insert(named_claude_key(name));
        }
        let (next, keys) = assemble(None, &copy.items, &copy.progress, Some(&copy.fixed))?;
        write_keys(self.storage.as_ref(), &names, &keys)?;
        let fires: Vec<Fire> = copy.progress.fires.iter().cloned().collect();
        self.storage.replace_fires(&fires)?;
        self.save(&next)?;
        *store = next;
        Ok(())
    }

    pub fn take_back(&self, changes: &Changes, recorded: &mut BTreeMap<Item, Recorded>, from: &Author) -> Result<Vec<Note>, String> {
        self.adopt_runs(&changes.runs)?;
        let mut notes = {
            let mut store = self.store.lock().unwrap();
            let (mut current, mut progress, _) = project(&store, self.storage.as_ref());
            let edits: BTreeMap<Item, Edit> = changes.edits.iter().filter(|(item, _)| !item.is_taste()).map(|(item, edit)| (item.clone(), edit.clone())).collect();
            let notes = crate::account_copy::merge(&mut current, recorded, &edits, from, &Self::normalize);
            progress.join(&changes.progress);
            let names = item_key_names(&store);
            let (next, keys) = assemble(Some(&store), &current, &progress, None)?;
            write_keys(self.storage.as_ref(), &names, &keys)?;
            let fires: Vec<Fire> = progress.fires.iter().cloned().collect();
            self.storage.replace_fires(&fires)?;
            self.save(&next)?;
            *store = next;
            notes
        };
        for set in &changes.run_sets {
            match self.run_set(&set.connector, &set.subject, &set.name, &set.value) {
                Ok(_) => {}
                Err(text) if text.contains("no run variables") => notes.push(Note { item: Item::workflow(&set.connector), text: format!("a run variable was dropped: {text}") }),
                Err(text) => return Err(text),
            }
        }
        Ok(notes)
    }

    pub fn normalize(item: &Item, version: Version) -> Version {
        let name = item.as_str();
        let value = if name.starts_with("workflow/") {
            round_trip::<Connector>(&version.value)
        } else if name == "linear" {
            round_trip::<crate::linear::Account>(&version.value)
        } else if name.starts_with("slack-bot/") {
            round_trip::<crate::slack::Bot>(&version.value)
        } else if name == "github" {
            round_trip::<crate::github::Account>(&version.value)
        } else if name == "vercel" {
            round_trip::<crate::vercel::Account>(&version.value)
        } else if name.starts_with("convex/") {
            round_trip::<crate::convex::Connection>(&version.value)
        } else if name.starts_with("mcp/") {
            round_trip::<crate::mcp::Entry>(&version.value)
        } else if item.is_taste() {
            return crate::taste::Tastes::normalize_copy_item(item, &version);
        } else {
            None
        };
        match value {
            Some(value) => Version { value, secrets: version.secrets },
            None => version,
        }
    }

    pub fn own_runs(&self) -> Vec<Value> {
        self.runs.lock().unwrap().iter().filter_map(|run| serde_json::to_value(run).ok()).collect()
    }

    /// Whether the store holds a live run of `connector` on `subject`, the same run `run_set` writes.
    pub fn has_run(&self, connector: &str, subject: &crate::agent::Subject) -> bool {
        self.runs.lock().unwrap().iter().any(|run| {
            matches!(run.status, runner::Status::Running | runner::Status::Waiting)
                && matches!(run.session, runner::Session::Live { .. })
                && &run.subject == subject
                && run.session_owner.as_deref().unwrap_or(run.connector.as_str()) == connector
        })
    }

    pub fn admit_only(&self, admits: fn(&Connector) -> bool) {
        let mut slot = self.admits.lock().unwrap();
        if slot.is_none() {
            *slot = Some(admits);
        }
    }

    pub fn runner(&self) -> Option<MachineId> {
        self.store.lock().unwrap().offline_runner.clone()
    }

    pub fn set_runner(&self, machine: Option<MachineId>) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        if store.offline_runner == machine {
            return Ok(());
        }
        store.offline_runner = machine;
        self.save(&store)
    }

    pub fn held_for(&self) -> Option<MachineId> {
        self.store.lock().unwrap().held_for.clone()
    }

    pub fn hold_for_runner(&self) -> Result<Option<MachineId>, String> {
        let mut store = self.store.lock().unwrap();
        if store.held_for.is_none() {
            if let Some(runner) = store.offline_runner.clone() {
                store.held_for = Some(runner);
                self.save(&store)?;
            }
        }
        Ok(store.held_for.clone())
    }

    pub fn release(&self) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        if store.held_for.is_none() {
            return Ok(());
        }
        store.held_for = None;
        self.save(&store)
    }

    pub fn applied_handback(&self, machine: &str) -> Option<uuid::Uuid> {
        self.store.lock().unwrap().handbacks.get(machine).copied()
    }

    pub fn remember_handback(&self, machine: MachineId, id: uuid::Uuid) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        if store.handbacks.get(&machine) == Some(&id) {
            return Ok(());
        }
        store.handbacks.insert(machine, id);
        self.save(&store)
    }

    fn adopt_runs(&self, incoming: &[Value]) -> Result<(), String> {
        if incoming.is_empty() {
            return Ok(());
        }
        {
            let mut runs = self.runs.lock().unwrap();
            for value in incoming {
                let run: runner::Run = serde_json::from_value(value.clone()).map_err(|error| format!("a handed-back run: {error}"))?;
                if runs.iter().any(|have| have.id == run.id) {
                    continue;
                }
                runs.push(run);
            }
        }
        self.save_runs()
    }
}

fn round_trip<T: DeserializeOwned + Serialize>(value: &Value) -> Option<Value> {
    serde_json::to_value(serde_json::from_value::<T>(without_order(value)).ok()?).ok()
}

fn project(store: &super::Store, storage: &dyn Storage) -> (BTreeMap<Item, Version>, Progress, Value) {
    let super::Store {
        linear,
        slack: _,
        slack_bots,
        slack_threads,
        github,
        vercel,
        // The PostHog key does not travel with an account copy.
        posthog: _,
        upstash,
        box_claude_tokens,
        boxes,
        box_asks,
        box_grants,
        repo_insight,
        mcp_servers,
        convex,
        box_released_at,
        apps_seen,
        linear_changes_from,
        change_counts,
        change_retries,
        connectors,
        offline_runner: _,
        held_for: _,
        handbacks: _,
    } = store;
    let mut items = BTreeMap::new();
    let mut workflows = BTreeMap::new();
    for (index, connector) in connectors.iter().enumerate() {
        if let Some(progress) = workflow_progress(connector) {
            workflows.insert(connector.id.clone(), progress);
        }
        let mut cleared = connector.clone();
        clear_progress(&mut cleared);
        let value = with_order(serde_json::to_value(&cleared).expect("workflow"), index);
        items.insert(Item::workflow(&connector.id), Version { value, secrets: BTreeMap::new() });
    }
    if let Some(account) = linear {
        items.insert(Item::integration("linear"), keyed(account, storage.key(LINEAR_KEY)));
    }
    for (index, bot) in slack_bots.iter().enumerate() {
        let value = with_order(serde_json::to_value(bot).expect("slack bot"), index);
        items.insert(Item::slack_bot(&bot.id), keyed_value(value, storage.key(&slack_bot_key(&bot.id))));
    }
    if let Some(account) = github {
        items.insert(Item::integration("github"), keyed(account, storage.key(GITHUB_KEY)));
    }
    if let Some(account) = vercel {
        items.insert(Item::integration("vercel"), keyed(account, storage.key(VERCEL_KEY)));
    }
    for (index, entry) in mcp_servers.iter().enumerate() {
        let (kind, names) = entry.server.transport.secrets();
        let prefix = if kind == "env" { "env" } else { "header" };
        let mut secrets = BTreeMap::new();
        for name in names {
            if let Some(value) = storage.key(&mcp_key(&entry.id, kind, name)).filter(|value| !value.is_empty()) {
                secrets.insert(format!("{prefix}:{name}"), value);
            }
        }
        let value = with_order(serde_json::to_value(entry).expect("mcp server"), index);
        items.insert(Item::mcp_server(&entry.id), Version { value, secrets });
    }
    for (index, connection) in convex.iter().enumerate() {
        let value = with_order(serde_json::to_value(connection).expect("convex"), index);
        items.insert(Item::convex(&connection.id), keyed_value(value, storage.key(&crate::convex::key_name(&connection.id))));
    }
    let progress = Progress {
        workflows,
        slack_threads: slack_threads.clone(),
        fires: storage.fires().into_iter().collect(),
        change_retries: change_retries.clone(),
        apps_seen: apps_seen.clone(),
        linear_changes_from: linear_changes_from.clone(),
    };
    let fixed = serde_json::to_value(Fixed {
        upstash: upstash.clone(),
        box_claude_tokens: box_claude_tokens.clone(),
        boxes: boxes.clone(),
        box_asks: box_asks.clone(),
        box_grants: box_grants.clone(),
        repo_insight: repo_insight.clone(),
        change_counts: change_counts.clone(),
        box_released_at: box_released_at.clone(),
    })
    .expect("fixed");
    (items, progress, fixed)
}

fn workflow_progress(connector: &Connector) -> Option<WorkflowProgress> {
    let progress = WorkflowProgress {
        since: connector.since.clone(),
        handled: connector.handled.iter().cloned().collect(),
        created: connector.created.clone(),
        followed: connector.followed.clone(),
        paged: connector.paged.clone(),
    };
    let empty = progress.since.is_empty() && progress.handled.is_empty() && progress.created.is_empty() && progress.followed.is_empty() && progress.paged.is_empty();
    (!empty).then_some(progress)
}

fn clear_progress(connector: &mut Connector) {
    connector.since.clear();
    connector.handled.clear();
    connector.created.clear();
    connector.followed.clear();
    connector.paged.clear();
}

fn keyed(value: impl Serialize, secret: Option<String>) -> Version {
    keyed_value(serde_json::to_value(&value).expect("account item"), secret)
}

fn keyed_value(value: Value, secret: Option<String>) -> Version {
    let mut secrets = BTreeMap::new();
    if let Some(secret) = secret.filter(|secret| !secret.is_empty()) {
        secrets.insert("key".into(), secret);
    }
    Version { value, secrets }
}

fn with_order(mut value: Value, order: usize) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.insert("order".into(), Value::from(order as u64));
    }
    value
}

fn without_order(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("order");
    }
    value
}

fn assemble(existing: Option<&super::Store>, items: &BTreeMap<Item, Version>, progress: &Progress, fixed: Option<&Value>) -> Result<(super::Store, BTreeMap<String, String>), String> {
    let mut store = match existing {
        Some(existing) => super::Store {
            upstash: existing.upstash.clone(),
            box_claude_tokens: existing.box_claude_tokens.clone(),
            boxes: existing.boxes.clone(),
            box_asks: existing.box_asks.clone(),
            box_grants: existing.box_grants.clone(),
            repo_insight: existing.repo_insight.clone(),
            change_counts: existing.change_counts.clone(),
            box_released_at: existing.box_released_at.clone(),
            offline_runner: existing.offline_runner.clone(),
            held_for: existing.held_for.clone(),
            handbacks: existing.handbacks.clone(),
            ..super::Store::default()
        },
        None => super::Store::default(),
    };
    if let Some(fixed) = fixed {
        let fixed: Fixed = if fixed.is_null() { Fixed::default() } else { serde_json::from_value(fixed.clone()).map_err(|error| format!("account copy fixed part: {error}"))? };
        store.upstash = fixed.upstash;
        store.box_claude_tokens = fixed.box_claude_tokens;
        store.boxes = fixed.boxes;
        store.box_asks = fixed.box_asks;
        store.box_grants = fixed.box_grants;
        store.repo_insight = fixed.repo_insight;
        store.change_counts = fixed.change_counts;
        store.box_released_at = fixed.box_released_at;
    }
    let keys = apply_items(&mut store, existing, items, progress)?;
    Ok((store, keys))
}

fn apply_items(store: &mut super::Store, existing: Option<&super::Store>, items: &BTreeMap<Item, Version>, progress: &Progress) -> Result<BTreeMap<String, String>, String> {
    let old_workflows = positions(existing.map(|store| store.connectors.iter().map(|connector| connector.id.clone()).collect()));
    let old_bots = positions(existing.map(|store| store.slack_bots.iter().map(|bot| bot.id.clone()).collect()));
    let old_mcp = positions(existing.map(|store| store.mcp_servers.iter().map(|entry| entry.id.clone()).collect()));
    let old_convex = positions(existing.map(|store| store.convex.iter().map(|connection| connection.id.clone()).collect()));
    let mut workflow_fallback = old_workflows.len() as u64;
    let mut bot_fallback = old_bots.len() as u64;
    let mut mcp_fallback = old_mcp.len() as u64;
    let mut convex_fallback = old_convex.len() as u64;
    let mut workflows = Vec::new();
    let mut bots = Vec::new();
    let mut servers = Vec::new();
    let mut deployments = Vec::new();
    let mut keys = BTreeMap::new();
    store.linear = None;
    store.github = None;
    store.vercel = None;
    for (item, version) in items {
        if item.is_taste() {
            continue;
        }
        let name = item.as_str();
        if let Some(id) = name.strip_prefix("workflow/") {
            let mut connector: Connector = parse(item, version)?;
            connector.id = id.to_owned();
            clear_progress(&mut connector);
            if let Some(part) = progress.workflows.get(id) {
                connector.since = part.since.clone();
                let mut handled: Vec<String> = part.handled.iter().cloned().collect();
                handled.sort();
                connector.handled = handled;
                connector.created = part.created.clone();
                connector.followed = part.followed.clone();
                connector.paged = part.paged.clone();
            }
            let order = next_order(&version.value, id, &old_workflows, &mut workflow_fallback);
            workflows.push((order, id.to_owned(), connector));
        } else if let Some(id) = name.strip_prefix("slack-bot/") {
            let mut bot: crate::slack::Bot = parse(item, version)?;
            bot.id = id.to_owned();
            if let Some(secret) = secret(version, "key") {
                keys.insert(slack_bot_key(id), secret);
            }
            let order = next_order(&version.value, id, &old_bots, &mut bot_fallback);
            bots.push((order, id.to_owned(), bot));
        } else if let Some(id) = name.strip_prefix("mcp/") {
            let mut entry: crate::mcp::Entry = parse(item, version)?;
            entry.id = id.to_owned();
            attach_secret_names(&mut entry, &version.secrets);
            for (role, value) in &version.secrets {
                if let Some(name) = mcp_secret_name(id, role) {
                    if !value.is_empty() {
                        keys.insert(name, value.clone());
                    }
                }
            }
            let order = next_order(&version.value, id, &old_mcp, &mut mcp_fallback);
            servers.push((order, id.to_owned(), entry));
        } else if let Some(id) = name.strip_prefix("convex/") {
            let mut connection: crate::convex::Connection = parse(item, version)?;
            connection.id = id.to_owned();
            if let Some(secret) = secret(version, "key") {
                keys.insert(crate::convex::key_name(id), secret);
            }
            let order = next_order(&version.value, id, &old_convex, &mut convex_fallback);
            deployments.push((order, id.to_owned(), connection));
        } else if name == "linear" {
            store.linear = Some(parse(item, version)?);
            if let Some(secret) = secret(version, "key") {
                keys.insert(LINEAR_KEY.to_owned(), secret);
            }
        } else if name == "github" {
            store.github = Some(parse(item, version)?);
            if let Some(secret) = secret(version, "key") {
                keys.insert(GITHUB_KEY.to_owned(), secret);
            }
        } else if name == "vercel" {
            store.vercel = Some(parse(item, version)?);
            if let Some(secret) = secret(version, "key") {
                keys.insert(VERCEL_KEY.to_owned(), secret);
            }
        } else {
            return Err(format!("this copy has {item}, which is not part of the connectors store"));
        }
    }
    sort_ranked(&mut workflows);
    sort_ranked(&mut bots);
    sort_ranked(&mut servers);
    sort_ranked(&mut deployments);
    store.connectors = workflows.into_iter().map(|(_, _, connector)| connector).collect();
    store.slack_bots = bots.into_iter().map(|(_, _, bot)| bot).collect();
    store.mcp_servers = servers.into_iter().map(|(_, _, entry)| entry).collect();
    store.convex = deployments.into_iter().map(|(_, _, connection)| connection).collect();
    store.slack = store.slack_bots.first().map(crate::slack::Bot::account);
    if let Some(first) = store.slack_bots.first() {
        if let Some(token) = keys.get(&slack_bot_key(&first.id)).cloned() {
            keys.insert(SLACK_KEY.to_owned(), token);
        }
    }
    store.slack_threads = progress.slack_threads.clone();
    store.apps_seen = progress.apps_seen.clone();
    store.linear_changes_from = progress.linear_changes_from.clone();
    store.change_retries = progress.change_retries.clone();
    Ok(keys)
}

fn sort_ranked<T>(ranked: &mut Vec<(u64, String, T)>) {
    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
}

fn positions(ids: Option<Vec<String>>) -> BTreeMap<String, u64> {
    ids.unwrap_or_default().into_iter().enumerate().map(|(index, id)| (id, index as u64)).collect()
}

fn next_order(value: &Value, id: &str, known: &BTreeMap<String, u64>, fallback: &mut u64) -> u64 {
    if let Some(order) = value.get("order").and_then(Value::as_u64) {
        return order;
    }
    if let Some(order) = known.get(id).copied() {
        return order;
    }
    let order = *fallback;
    *fallback += 1;
    order
}

fn parse<T: DeserializeOwned>(item: &Item, version: &Version) -> Result<T, String> {
    serde_json::from_value(without_order(&version.value)).map_err(|error| format!("{item}: {error}"))
}

fn secret(version: &Version, role: &str) -> Option<String> {
    version.secrets.get(role).filter(|value| !value.is_empty()).cloned()
}

fn mcp_secret_name(id: &str, role: &str) -> Option<String> {
    let (prefix, name) = role.split_once(':')?;
    let kind = match prefix {
        "env" => "env",
        "header" => "headers",
        _ => return None,
    };
    Some(mcp_key(id, kind, name))
}

fn attach_secret_names(entry: &mut crate::mcp::Entry, secrets: &BTreeMap<String, String>) {
    let listed = entry.server.transport.secrets().1.to_vec();
    let kind = entry.server.transport.secrets().0;
    let prefix = if kind == "env" { "env" } else { "header" };
    let extra: Vec<String> = secrets
        .keys()
        .filter_map(|role| {
            let (got, name) = role.split_once(':')?;
            (got == prefix && !listed.iter().any(|have| have == name)).then(|| name.to_owned())
        })
        .collect();
    if extra.is_empty() {
        return;
    }
    match &mut entry.server.transport {
        crate::mcp::Reach::Stdio { env, .. } => env.extend(extra),
        crate::mcp::Reach::Http { headers, .. } | crate::mcp::Reach::Sse { headers, .. } => headers.extend(extra),
    }
}

fn item_key_names(store: &super::Store) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if store.linear.is_some() {
        names.insert(LINEAR_KEY.to_owned());
    }
    for bot in &store.slack_bots {
        names.insert(slack_bot_key(&bot.id));
    }
    if !store.slack_bots.is_empty() || store.slack.is_some() {
        names.insert(SLACK_KEY.to_owned());
    }
    if store.github.is_some() {
        names.insert(GITHUB_KEY.to_owned());
    }
    if store.vercel.is_some() {
        names.insert(VERCEL_KEY.to_owned());
    }
    for entry in &store.mcp_servers {
        let (kind, listed) = entry.server.transport.secrets();
        for name in listed {
            names.insert(mcp_key(&entry.id, kind, name));
        }
    }
    for connection in &store.convex {
        names.insert(crate::convex::key_name(&connection.id));
    }
    names
}

fn write_keys(storage: &dyn Storage, previous: &BTreeSet<String>, keys: &BTreeMap<String, String>) -> Result<(), String> {
    for (name, value) in keys {
        storage.write_key(name, value)?;
    }
    for name in previous {
        if !keys.contains_key(name) {
            storage.remove_key(name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_copy::{Author, Changes, Edit, Item, Version};
    use crate::connectors::{Columns, Connector, Connectors, Effort, FileStorage, Filed, Fix, Followed, Paged, PermissionMode, RunsOn, Storage, Trigger, Work};
    use crate::machine::refuse_real_home;
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
    }

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("pinned-copy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn workflow(id: &str, name: &str, folder: &str) -> Connector {
        Connector {
            id: id.into(),
            name: name.into(),
            enabled: true,
            trigger: Trigger::Manual,
            folder: folder.into(),
            model: String::new(),
            effort: Effort::Medium,
            permission_mode: PermissionMode::AcceptEdits,
            worktree: false,
            instructions: String::new(),
            columns: Columns::default(),
            since: String::new(),
            handled: vec![],
            team: String::new(),
            label: String::new(),
            fix: Fix::Off,
            created: BTreeMap::new(),
            followed: BTreeMap::new(),
            paged: BTreeMap::new(),
            proof: true,
            ask: None,
            runs_on: RunsOn::Any,
            repo: String::new(),
            account: String::new(),
            slack_bot: String::new(),
            graph: None,
        }
    }

    fn bot(id: &str, name: &str, token: &str) -> crate::slack::Bot {
        crate::slack::Bot { id: id.into(), team_id: "T1".into(), team: "Acme".into(), user_id: id.into(), name: name.into(), app_id: String::new(), key_hint: crate::upstash::hint(token) }
    }

    fn write_key(dir: &Path, name: &str, value: &str) {
        FileStorage::new(dir.to_owned()).write_key(name, value).unwrap();
    }

    fn seeded(name: &str) -> (Connectors, PathBuf) {
        let dir = scratch(name);
        let mut zebra = workflow("c2", "Zebra", "/work/zebra");
        zebra.since = "2026-01-01T00:00:00Z".into();
        zebra.handled = vec!["ISSUE-1".into()];
        zebra.created.insert("C9:1".into(), Filed { id: "i1".into(), identifier: "ENG-1".into(), url: "https://linear.app/eng-1".into() });
        zebra.followed.insert("C9:1".into(), Followed { cursor: "2".into(), seen: Some("3".into()), replied: true });
        zebra.paged.insert("C9:1".into(), Paged { at: "2026-01-02T00:00:00Z".into(), ping: true });
        let alpha = workflow("c1", "Alpha", "/work/alpha");
        let second = "xoxb-round-trip-second-SSSS";
        let first = "xoxb-round-trip-first-FFFF";
        let store = json!({
            "linear": crate::linear::Account { name: "Ana".into(), email: "ana@acme.dev".into(), organization: "Acme".into() },
            "slack": crate::slack::Account { team: "Acme".into(), user_id: "U2".into(), name: "Second".into() },
            "slack_bots": [bot("U2", "Second", second), bot("U1", "First", first)],
            "slack_threads": {"C9:1": "U2"},
            "github": crate::github::Account { login: "ana".into(), name: "Ana".into() },
            "vercel": crate::vercel::Account { key_hint: "…rcel".into() },
            "upstash": crate::upstash::Account { key_hint: "…box1".into(), claude_hint: "…tok1".into() },
            "connectors": [zebra, alpha],
            "mcp_servers": [
                crate::mcp::Entry { id: "m2".into(), enabled: true, server: crate::mcp::Spec { name: "docs".into(), project: None, transport: crate::mcp::Reach::Stdio { command: "npx".into(), args: vec![], env: vec!["API_KEY".into()] } } },
                crate::mcp::Entry { id: "m1".into(), enabled: true, server: crate::mcp::Spec { name: "remote".into(), project: None, transport: crate::mcp::Reach::Http { url: "https://example.com/mcp".into(), headers: vec!["Authorization".into()] } } },
            ],
            "convex": [crate::convex::Connection { id: "quick-lemur-789".into(), kind: crate::convex::Kind::Dev, url: "https://quick-lemur-789.convex.cloud".into(), team: "acme".into(), project: "site".into(), key_hint: "…CCCC".into(), repos: vec!["github.com/acme/site".into()] }],
            "repo_insight": {"on": true},
            "apps_seen": {"c2": "C9:1"},
            "linear_changes_from": "cursor-9",
            "change_counts": {"c2": {"skippedAway": 2}},
            "change_retries": {"c2": {"k": "2026-01-03T00:00:00.000Z"}},
            "box_released_at": {"c2": {"issue-1": 10}},
        });
        std::fs::write(dir.join("connectors.json"), serde_json::to_vec_pretty(&store).unwrap()).unwrap();
        write_key(&dir, "linear-key", "lin_api_round-trip-LLLL");
        write_key(&dir, "github-key", "ghp_round-trip-GGGG");
        write_key(&dir, "vercel-key", "vck_round-trip-VVVV");
        write_key(&dir, "slack-key", second);
        write_key(&dir, "slack-key:U2", second);
        write_key(&dir, "slack-key:U1", first);
        write_key(&dir, "convex-key-quick-lemur-789", "dev:quick-lemur-789|round-trip-convex-CCCC");
        write_key(&dir, "mcp:m2:env:API_KEY", "mcp-env-round-trip-EEEE");
        write_key(&dir, "mcp:m1:headers:Authorization", "mcp-header-round-trip-HHHH");
        std::fs::write(dir.join("fires.json"), serde_json::to_vec(&json!([{"workflow": "c2", "key": "issue-1", "claimer": "hub", "at": 1_700_000_000}])).unwrap()).unwrap();
        let connectors = Connectors::new(FileStorage::new(dir.clone()), "pinned".into());
        (connectors, dir)
    }

    fn mode(path: &Path) -> u32 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            return std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            0o600
        }
    }

    fn contains_secret(dir: &Path, secret: &str) -> bool {
        std::fs::read_dir(dir).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap_or_default();
            bytes.windows(secret.len()).any(|window| window == secret.as_bytes())
        })
    }

    #[test]
    fn copy_parts_then_take_copy_matches_the_store_and_writes_keys_0600() {
        refuse_real_home();
        let (connectors, dir) = seeded("round");
        let dest = scratch("round-back");
        let (items, progress, fixed) = connectors.copy_parts();
        let copy = Copy { items, progress, fixed, runner: None };
        let back = Connectors::new(FileStorage::new(dest.clone()), "pinned".into());
        back.take_copy(&copy).unwrap();
        let (before, before_keys) = connectors.export();
        let (after, after_keys) = back.export();
        assert_eq!(after["connectors"][0]["id"], "c2");
        assert_eq!(after["connectors"][1]["id"], "c1");
        assert_eq!(after["slack_bots"][0]["id"], "U2");
        assert_eq!(after["slack"]["userId"], "U2");
        assert_eq!(after["mcp_servers"][0]["id"], "m2");
        assert_eq!(before, after);
        assert_eq!(before_keys, after_keys);
        assert_eq!(connectors.copy_parts().1.fires, back.copy_parts().1.fires);
        assert!(back.copy_parts().1.fires.iter().any(|fire| fire.workflow == "c2" && fire.key == "issue-1" && fire.at == 1_700_000_000));
        for name in ["linear-key", "github-key", "vercel-key", "slack-key", "slack-key%3AU2", "slack-key%3AU1", "convex-key-quick-lemur-789", "mcp%3Am2%3Aenv%3AAPI_KEY", "mcp%3Am1%3Aheaders%3AAuthorization"] {
            let path = dest.join(name);
            assert_eq!(mode(&path), 0o600, "{name}");
            assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(dir.join(name)).unwrap(), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn take_copy_leaves_no_upstash_key_no_claude_token_and_drops_a_missing_key() {
        refuse_real_home();
        let dir = scratch("drops");
        let store = json!({
            "linear": {"name": "Ana", "email": "ana@acme.dev", "organization": "Acme"},
            "upstash": {"key_hint": "…UUUU", "claude_hint": "…CCCC"},
            "box_claude_tokens": {"work": "…CCCC"},
            "connectors": [],
        });
        std::fs::write(dir.join("connectors.json"), serde_json::to_vec(&store).unwrap()).unwrap();
        write_key(&dir, "linear-key", "lin_api_drop-me-LLLL");
        write_key(&dir, "upstash-box-key", "box_drop-me-UUUU");
        write_key(&dir, "claude-oauth-token:work", "sk-ant-oat01-drop-me-CCCC");
        let connectors = Connectors::new(FileStorage::new(dir.clone()), "pinned".into());
        let (mut items, progress, fixed) = connectors.copy_parts();
        items.remove(&Item::integration("linear"));
        connectors.take_copy(&Copy { items, progress, fixed, runner: None }).unwrap();
        let (store, keys) = connectors.export();
        assert_eq!(store["linear"], Value::Null);
        assert_eq!(store["upstash"]["key_hint"], "…UUUU");
        assert_eq!(store["box_claude_tokens"]["work"], "…CCCC");
        assert_eq!(keys.linear, None);
        assert!(!contains_secret(&dir, "lin_api_drop-me-LLLL"));
        assert!(!contains_secret(&dir, "box_drop-me-UUUU"));
        assert!(!contains_secret(&dir, "sk-ant-oat01-drop-me-CCCC"));
        assert!(!dir.join("linear-key").exists());
        assert!(!dir.join("upstash-box-key").exists());
        assert!(!dir.join("claude-oauth-token%3Awork").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stop_finding_checks_nothing() {
        refuse_real_home();
        let dir = scratch("stop-finding");
        std::fs::write(dir.join("connectors.json"), br#"{"connectors":[]}"#).unwrap();
        let connectors = Connectors::new(FileStorage::new(dir.clone()), "pinned".into());
        connectors.stop_finding();
        let called = AtomicBool::new(false);
        let open = |_work: Work| {
            called.store(true, Ordering::SeqCst);
            async { Ok::<(), String>(()) }
        };
        let _ = block_on(connectors.check_once(&open));
        assert!(!called.load(Ordering::SeqCst));
        assert_eq!(connectors.checks_open(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_held_store_checks_nothing_and_refuses_run_now() {
        refuse_real_home();
        let dir = scratch("held");
        let store = json!({
            "held_for": "macbook",
            "offline_runner": "macbook",
            "connectors": [workflow("c1", "Triage", "/work")],
        });
        std::fs::write(dir.join("connectors.json"), serde_json::to_vec_pretty(&store).unwrap()).unwrap();
        let connectors = Connectors::new(FileStorage::new(dir.clone()), "pinned".into());
        assert_eq!(connectors.hold_for_runner().unwrap().as_deref(), Some("macbook"));
        assert_eq!(connectors.get_settings().workflows_held_for.as_deref(), Some("macbook"));
        assert_eq!(connectors.get_settings().offline_runner.as_deref(), Some("macbook"));
        let called = AtomicBool::new(false);
        let open = |_work: Work| {
            called.store(true, Ordering::SeqCst);
            async { Err::<(), String>("a held store made a request".into()) }
        };
        let check = block_on(connectors.check_once(&open));
        assert!(!called.load(Ordering::SeqCst));
        assert_eq!(check.error.as_deref(), Some("workflows wait for macbook to hand back what it ran while the hub was off"));
        assert_eq!(connectors.get_settings().checked.unwrap().error.as_deref(), Some("workflows wait for macbook to hand back what it ran while the hub was off"));
        assert_eq!(connectors.run_now("c1", "Ana").unwrap_err(), "workflows wait for macbook to hand back what it ran while the hub was off");
        assert_eq!(block_on(connectors.on_tabs("mac", 1, &[], |_work: Work| async { Err::<(), String>("a held store drove a tab".into()) })).unwrap_err(), "workflows wait for macbook to hand back what it ran while the hub was off");
        connectors.release().unwrap();
        assert_eq!(connectors.held_for(), None);
        let saved: Value = serde_json::from_slice(&std::fs::read(dir.join("connectors.json")).unwrap()).unwrap();
        assert!(saved.get("held_for").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn take_back_keeps_the_later_name_and_joins_handled() {
        refuse_real_home();
        let (connectors, dir) = seeded("back");
        let (items, mut progress, _) = connectors.copy_parts();
        let item = Item::workflow("c1");
        let current = items.get(&item).unwrap().clone();
        let mut renamed = current.clone();
        renamed.value["name"] = json!("Renamed");
        progress.workflows.entry("c1".into()).or_default().handled.insert("ISSUE-9".into());
        let edits = BTreeMap::from([(item, Edit { at: 5, base: Some(current), ours: Some(renamed) })]);
        let changes = Changes { edits, progress, ..Changes::default() };
        let mut recorded = BTreeMap::new();
        let from = Author { id: "m".into(), name: "Mac".into() };
        let notes = connectors.take_back(&changes, &mut recorded, &from).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let again = connectors.take_back(&changes, &mut recorded, &from).unwrap();
        assert!(again.is_empty(), "{again:?}");
        let (store, _) = connectors.export();
        let connectors_json = store["connectors"].as_array().unwrap();
        assert_eq!(connectors_json.len(), 2);
        assert_eq!(connectors_json[0]["id"], "c2");
        assert_eq!(connectors_json[1]["id"], "c1");
        assert_eq!(connectors_json[1]["name"], "Renamed");
        assert_eq!(connectors_json[1]["handled"], json!(["ISSUE-9"]));
        assert_eq!(connectors_json[0]["since"], "2026-01-01T00:00:00Z");
        assert_eq!(store["slack_bots"][0]["id"], "U2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_fills_a_workflows_defaulted_proof() {
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
        let normalized = Connectors::normalize(&Item::workflow("triage"), Version { value: old.clone(), secrets: BTreeMap::new() });
        assert_eq!(normalized.value["proof"], true);
        assert_ne!(normalized.value, old);
    }

    #[test]
    fn key_debug_shows_the_tail() {
        refuse_real_home();
        let keys = crate::connectors::Keys { linear: Some("lin_api_super-secret-linear-key-XYZ9".into()), slack: Some("xoxb-super-secret-slack".into()), ..crate::connectors::Keys::default() };
        let shown = format!("{keys:?}");
        assert!(!shown.contains("lin_api_super-secret-linear-key-XYZ9"));
        assert!(!shown.contains("xoxb-super-secret-slack"));
        assert!(shown.contains("…XYZ9"));
        let secrets = crate::mcp::Secrets { env: BTreeMap::from([("API_KEY".into(), "mcp-secret-value-QQQQ".into())]), ..crate::mcp::Secrets::default() };
        let shown = format!("{secrets:?}");
        assert!(!shown.contains("mcp-secret-value-QQQQ"));
        assert!(shown.contains("…QQQQ"));
        let upstash = crate::upstash::Keys::parse("box_super-secret-key-ZZZ1", "sk-ant-oat01-super-secret-YYYY").unwrap();
        let shown = format!("{upstash:?}");
        assert!(!shown.contains("super-secret"));
        assert!(shown.contains("…ZZZ1"));
        assert!(shown.contains("…YYYY"));
    }
}
