//! One JSON document per released hub version, and the diff that says what an older app would lose.
//! Each root stores what that version sends and what it accepts. One schema cannot say both.

use crate::account::{
    Failure, NewMachine, NewPassword, Paired, Pairing, Registered, Session, SignIn,
};
use crate::connectors::{Connector, Settings, Suggestions};
use crate::wire::{
    AccountView, ClientFrame, MachineFrame, MachineView, PairCode, ProjectView, TabView, ToClient,
    ToMachine,
};
use crate::{github, linear, slack, sync, upstash, vercel};
use schemars::generate::SchemaSettings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// `crates/core/wire-contract`, from this crate's manifest so a server binary finds it too.
pub fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("wire-contract")
}

/// What one root sends and what it accepts. Serialize mode marks a `serde(default)` field required,
/// because the sender writes it. Deserialize mode leaves it optional, because the reader fills it in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Schemas {
    pub send: Value,
    pub accept: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Contract {
    pub version: String,
    pub routes: Vec<String>,
    pub inbound: BTreeMap<String, Schemas>,
    pub outbound: BTreeMap<String, Schemas>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Break {
    pub version: String,
    pub direction: String,
    pub path: String,
    pub kind: String,
    pub impact: String,
}

pub const CURRENT_VERSION: &str = "current";

pub const STALE: &str = "wire contract is stale, run `cargo run -p pinned-server --bin wire-contract -- snapshot current`";

/// The contract of the code being compiled.
pub fn current(version: &str, routes: &[&str]) -> Contract {
    let mut inbound = BTreeMap::new();
    let mut outbound = BTreeMap::new();
    inbound.insert("ClientFrame".into(), schemas::<ClientFrame>());
    inbound.insert("MachineFrame".into(), schemas::<MachineFrame>());
    inbound.insert("SignIn".into(), schemas::<SignIn>());
    inbound.insert("NewPassword".into(), schemas::<NewPassword>());
    inbound.insert("NewMachine".into(), schemas::<NewMachine>());
    inbound.insert("Pairing".into(), schemas::<Pairing>());
    outbound.insert("ToClient".into(), schemas::<ToClient>());
    outbound.insert("ToMachine".into(), schemas::<ToMachine>());
    outbound.insert("Session".into(), schemas::<Session>());
    outbound.insert("AccountView".into(), schemas::<AccountView>());
    outbound.insert("Registered".into(), schemas::<Registered>());
    outbound.insert("Paired".into(), schemas::<Paired>());
    outbound.insert("Failure".into(), schemas::<Failure>());
    outbound.insert("MachineView".into(), schemas::<MachineView>());
    outbound.insert("PairCode".into(), schemas::<PairCode>());
    outbound.insert("TabView".into(), schemas::<TabView>());
    outbound.insert("ProjectView".into(), schemas::<ProjectView>());
    // Hub replies travel as `serde_json::Value`. These are the named structs a handler builds one from.
    outbound.insert("Settings".into(), schemas::<Settings>());
    outbound.insert("LinearAccount".into(), schemas::<linear::Account>());
    outbound.insert("Suggestions".into(), schemas::<Suggestions>());
    outbound.insert("SlackAccount".into(), schemas::<slack::Account>());
    outbound.insert("Connector".into(), schemas::<Connector>());
    outbound.insert("SyncProject".into(), schemas::<sync::Project>());
    outbound.insert("SyncEntry".into(), schemas::<sync::Entry>());
    outbound.insert("SyncFile".into(), schemas::<sync::File>());
    outbound.insert("GithubAccount".into(), schemas::<github::Account>());
    outbound.insert("GithubRepo".into(), schemas::<github::Repo>());
    outbound.insert("VercelAccount".into(), schemas::<vercel::Account>());
    outbound.insert("VercelModel".into(), schemas::<vercel::Model>());
    outbound.insert("UpstashAccount".into(), schemas::<upstash::Account>());
    Contract {
        version: version.into(),
        routes: routes.iter().map(|route| (*route).to_string()).collect(),
        inbound,
        outbound,
    }
}

fn schemas<T: JsonSchema>() -> Schemas {
    Schemas {
        send: schema::<T>(true),
        accept: schema::<T>(false),
    }
}

#[cfg(test)]
fn schema_de<T: JsonSchema>() -> Value {
    schema::<T>(false)
}

#[cfg(test)]
fn schema_ser<T: JsonSchema>() -> Value {
    schema::<T>(true)
}

fn schema<T: JsonSchema>(serialize: bool) -> Value {
    let mut settings = SchemaSettings::draft2020_12();
    // Untagged variants then carry their Rust name, which the diff uses when there is no tag.
    settings.untagged_enum_variant_titles = true;
    let settings = if serialize {
        settings.for_serialize()
    } else {
        settings.for_deserialize()
    };
    serde_json::to_value(settings.into_generator().into_root_schema_for::<T>()).expect("schema")
}

pub fn store(contract: &Contract) -> Result<PathBuf, String> {
    let dir = snapshot_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", contract.version));
    let body = serde_json::to_string_pretty(contract).map_err(|e| e.to_string())?;
    std::fs::write(&path, body + "\n").map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn load(path: &Path) -> Result<Contract, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// `crates/core` sits two directories under the workspace.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

pub fn current_path() -> PathBuf {
    snapshot_dir().join(format!("{CURRENT_VERSION}.json"))
}

/// `committed` is `current.json`. A missing file or any difference is the same failure.
pub fn require_fresh(generated: &Contract, committed: Option<&Contract>) -> Result<(), String> {
    if committed.is_some_and(|found| found == generated) {
        Ok(())
    } else {
        Err(STALE.into())
    }
}

pub fn collect(snapshots: &[Contract], hub: &Contract) -> Vec<Break> {
    let mut breaks = Vec::new();
    for old in snapshots {
        breaks.extend(diff(old, hub));
    }
    sort_breaks(&mut breaks);
    breaks
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    pub new_breaks: Vec<Break>,
    pub shipped: Vec<Break>,
}

/// Breaks in `head` that `base` does not already have. The key is version, direction, path, and kind.
/// `name_status` is `git diff --name-status` for `crates/core/wire-contract`.
pub fn classify(head: &[Break], base: &[Break], name_status: &str) -> Delta {
    let known: BTreeSet<_> = base.iter().map(Break::key).collect();
    let mut new_breaks = Vec::new();
    let mut shipped = Vec::new();
    for item in head {
        if known.contains(&item.key()) {
            shipped.push(item.clone());
        } else {
            new_breaks.push(item.clone());
        }
    }
    new_breaks.extend(snapshot_changes(name_status));
    sort_breaks(&mut new_breaks);
    sort_breaks(&mut shipped);
    Delta {
        new_breaks,
        shipped,
    }
}

impl Break {
    fn key(&self) -> (String, String, String, String) {
        (
            self.version.clone(),
            self.direction.clone(),
            self.path.clone(),
            self.kind.clone(),
        )
    }
}

/// Deleting or editing a version snapshot between the base and this tree is a new break.
/// `current.json` is the live contract, so a change to it is not one.
pub fn snapshot_changes(name_status: &str) -> Vec<Break> {
    let mut breaks = Vec::new();
    for line in name_status.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let Some(status) = parts.next() else {
            continue;
        };
        let Some(path) = parts.next() else {
            continue;
        };
        let kind = if status.starts_with('D') || status.starts_with('R') {
            "deleted_snapshot"
        } else if status.starts_with('M') || status.starts_with('T') {
            "edited_snapshot"
        } else {
            continue;
        };
        let Some(version) = snapshot_version(path) else {
            continue;
        };
        let impact = if kind == "deleted_snapshot" {
            format!("Deleting `{version}` hides a version this hub must still speak.")
        } else {
            format!("Editing `{version}` changes the recorded wire of that version.")
        };
        breaks.push(Break {
            version: version.into(),
            direction: "snapshot".into(),
            path: path.into(),
            kind: kind.into(),
            impact,
        });
    }
    breaks
}

fn snapshot_version(path: &str) -> Option<&str> {
    let name = path.rsplit(['/', '\\']).next()?;
    let version = name.strip_suffix(".json")?;
    if !version.contains('.') {
        return None;
    }
    if version
        .split('.')
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        Some(version)
    } else {
        None
    }
}

fn sort_breaks(breaks: &mut [Break]) {
    breaks.sort_by(|a, b| {
        version_key(&a.version)
            .cmp(&version_key(&b.version))
            .then((&a.direction, &a.path, &a.kind).cmp(&(&b.direction, &b.path, &b.kind)))
    });
}

fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Inbound, an old app's send schema must parse on this hub's accept schema.
/// Outbound, this hub's send schema must parse on an old app's accept schema.
pub fn diff(old: &Contract, new: &Contract) -> Vec<Break> {
    let mut breaks = vec![];
    for route in &old.routes {
        if !new.routes.iter().any(|kept| kept == route) {
            breaks.push(Break {
                version: old.version.clone(),
                direction: "route".into(),
                path: route.clone(),
                kind: "removed_route".into(),
                impact: format!(
                    "apps {} call `{route}`; this hub no longer serves it",
                    old.version
                ),
            });
        }
    }
    walk_map("inbound", old, &old.inbound, &new.inbound, &mut breaks);
    walk_map("outbound", old, &old.outbound, &new.outbound, &mut breaks);
    breaks.sort_by(|a, b| {
        (&a.version, &a.direction, &a.path, &a.kind).cmp(&(
            &b.version,
            &b.direction,
            &b.path,
            &b.kind,
        ))
    });
    breaks
}

fn walk_map(
    direction: &str,
    old: &Contract,
    before: &BTreeMap<String, Schemas>,
    after: &BTreeMap<String, Schemas>,
    breaks: &mut Vec<Break>,
) {
    for (name, old_schemas) in before {
        let path = name.clone();
        match after.get(name) {
            None => breaks.push(missing_root(direction, old, &path)),
            Some(new_schemas) => {
                let (earlier, later) = if direction == "inbound" {
                    (&old_schemas.send, &new_schemas.accept)
                } else {
                    (&old_schemas.accept, &new_schemas.send)
                };
                walk(
                    direction,
                    &old.version,
                    &path,
                    &node(earlier, earlier),
                    &node(later, later),
                    breaks,
                );
            }
        }
    }
}

fn missing_root(direction: &str, old: &Contract, path: &str) -> Break {
    let (kind, impact) = if direction == "inbound" {
        (
            "removed_variant",
            format!("apps {} send `{path}`; this hub rejects it", old.version),
        )
    } else {
        (
            "removed_variant",
            format!(
                "apps {} read `{path}`; this hub no longer sends it",
                old.version
            ),
        )
    };
    Break {
        version: old.version.clone(),
        direction: direction.into(),
        path: path.into(),
        kind: kind.into(),
        impact,
    }
}

pub fn report(breaks: &[Break]) -> String {
    let mut out = String::from("# Wire compatibility\n\n");
    if breaks.is_empty() {
        out.push_str(
            "No breaks. This hub still speaks every snapshot in `crates/core/wire-contract`.\n",
        );
        return out;
    }
    out.push_str(
        "A hub release must keep working for every snapshotted app. These changes do not:\n",
    );
    write_breaks(&mut out, breaks, "##");
    out
}

pub fn report_delta(delta: &Delta) -> String {
    let mut out = String::from("# Wire compatibility\n\n## New breaks\n");
    if delta.new_breaks.is_empty() {
        out.push_str("\nNo new breaks.\n");
    } else {
        write_breaks(&mut out, &delta.new_breaks, "###");
    }
    out.push_str("\n## Already shipped\n");
    if delta.shipped.is_empty() {
        out.push_str("\nNone.\n");
    } else {
        write_breaks(&mut out, &delta.shipped, "###");
    }
    out
}

fn write_breaks(out: &mut String, breaks: &[Break], heading: &str) {
    let mut version = "";
    for item in breaks {
        if item.version != version {
            version = &item.version;
            out.push_str(&format!("\n{heading} {}\n\n", item.version));
        }
        out.push_str(&format!(
            "- **{}** `{}` ({}): {}\n",
            item.direction, item.path, item.kind, item.impact
        ));
    }
}

#[derive(Clone, Debug)]
enum Node {
    Union(Vec<(String, Node)>),
    Object(BTreeMap<String, Field>),
    Array(Box<Node>),
    Map(Box<Node>),
    Leaf(String),
}

#[derive(Clone, Debug)]
struct Field {
    required: bool,
    node: Node,
}

fn node(schema: &Value, root: &Value) -> Node {
    let flat = concrete(schema, root, &mut Vec::new());
    if let Some(branches) = union_branches(&flat) {
        let parent = flat.get("properties").is_some().then(|| strip_union(&flat));
        let mut items = Vec::new();
        for branch in branches {
            let resolved = concrete(&branch, root, &mut Vec::new());
            // Name the branch before folding in a flattened parent, or every arm inherits the parent's tag.
            if let Some(values) = string_enum(&resolved) {
                for value in values {
                    items.push((value, Node::Leaf("string".into())));
                }
                continue;
            }
            let name = branch_name(&resolved, root);
            let merged = match &parent {
                Some(parent) => merge_objects(parent, &resolved),
                None => resolved,
            };
            items.push((name, node(&merged, root)));
        }
        return Node::Union(items);
    }
    if let Some(values) = string_enum(&flat) {
        return Node::Union(
            values
                .into_iter()
                .map(|value| (value, Node::Leaf("string".into())))
                .collect(),
        );
    }
    if let Some(items) = array_items(&flat, root) {
        return Node::Array(Box::new(items));
    }
    if let Some(values) = map_values(&flat, root) {
        return Node::Map(Box::new(values));
    }
    if flat.get("properties").is_some()
        || flat.get("type").and_then(|t| t.as_str()) == Some("object")
            && flat.get("additionalProperties").is_none()
    {
        return Node::Object(fields_of(&flat, root));
    }
    Node::Leaf(leaf(&flat))
}

fn fields_of(schema: &Value, root: &Value) -> BTreeMap<String, Field> {
    let required = required_names(schema);
    let mut fields = BTreeMap::new();
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return fields;
    };
    for (name, prop) in props {
        if prop.get("const").is_some() {
            continue;
        }
        let resolved = concrete(prop, root, &mut Vec::new());
        fields.insert(
            name.clone(),
            Field {
                required: required.contains(name),
                node: node(&resolved, root),
            },
        );
    }
    fields
}

fn required_names(schema: &Value) -> BTreeSet<String> {
    schema
        .get("required")
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

fn union_branches(schema: &Value) -> Option<Vec<Value>> {
    let branches = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))?
        .as_array()?
        .clone();
    if branches.iter().all(|b| simple_type(b).is_some()) {
        return None;
    }
    Some(branches)
}

fn nullable_union(schema: &Value) -> Option<String> {
    let branches = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))?
        .as_array()?;
    let mut names = branches
        .iter()
        .map(simple_type)
        .collect::<Option<Vec<_>>>()?;
    names.sort();
    names.dedup();
    Some(names.join("|"))
}

fn simple_type(schema: &Value) -> Option<String> {
    if schema.get("properties").is_some()
        || schema.get("oneOf").is_some()
        || schema.get("anyOf").is_some()
        || schema.get("enum").is_some()
    {
        return None;
    }
    match schema.get("type")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let mut names: Vec<_> = items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::to_owned)
                .collect();
            if names.is_empty() {
                return None;
            }
            names.sort();
            Some(names.join("|"))
        }
        _ => None,
    }
}

fn string_enum(schema: &Value) -> Option<Vec<String>> {
    let values = schema.get("enum")?.as_array()?;
    values
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect()
}

fn branch_name(branch: &Value, root: &Value) -> String {
    let branch = concrete(branch, root, &mut Vec::new());
    if let Some(tag) = tag_const(&branch) {
        return tag;
    }
    if let Some(value) = branch.get("const").and_then(|c| c.as_str()) {
        return value.to_owned();
    }
    if let Some(title) = branch.get("title").and_then(|t| t.as_str()) {
        if !title.is_empty() {
            return title.to_owned();
        }
    }
    match branch.get("properties").and_then(|p| p.as_object()) {
        Some(props) if !props.is_empty() => {
            let mut keys: Vec<_> = props.keys().cloned().collect();
            keys.sort();
            keys.join("+")
        }
        _ => leaf(&branch),
    }
}

fn tag_const(schema: &Value) -> Option<String> {
    let props = schema.get("properties")?.as_object()?;
    for key in ["cmd", "type", "kind", "op", "step"] {
        if let Some(value) = props.get(key).and_then(const_str) {
            return Some(value);
        }
    }
    None
}

fn const_str(schema: &Value) -> Option<String> {
    schema.get("const")?.as_str().map(str::to_owned)
}

fn strip_union(schema: &Value) -> Value {
    let mut kept = schema.clone();
    if let Some(obj) = kept.as_object_mut() {
        obj.remove("oneOf");
        obj.remove("anyOf");
    }
    kept
}

fn array_items(schema: &Value, root: &Value) -> Option<Node> {
    let typed = schema.get("type").and_then(|t| t.as_str()) == Some("array")
        || schema.get("prefixItems").is_some()
        || schema.get("items").is_some();
    if !typed {
        return None;
    }
    if let Some(prefix) = schema.get("prefixItems").and_then(|p| p.as_array()) {
        let mut items = Vec::new();
        for (i, item) in prefix.iter().enumerate() {
            items.push((i.to_string(), node(item, root)));
        }
        return Some(Node::Union(items));
    }
    Some(match schema.get("items") {
        Some(items) => node(items, root),
        None => Node::Leaf("any".into()),
    })
}

fn map_values(schema: &Value, root: &Value) -> Option<Node> {
    if schema.get("type").and_then(|t| t.as_str()) != Some("object") {
        return None;
    }
    let extra = schema.get("additionalProperties")?;
    if extra.as_bool() == Some(true) || schema.get("properties").is_some() {
        return None;
    }
    Some(node(extra, root))
}

fn leaf(schema: &Value) -> String {
    if schema.as_bool() == Some(true) || schema.as_object().is_some_and(|o| o.is_empty()) {
        return "any".into();
    }
    if let Some(joined) = nullable_union(schema) {
        return joined;
    }
    if let Some(simple) = simple_type(schema) {
        return annotate(schema, simple);
    }
    if let Some(values) = schema.get("enum").and_then(|e| e.as_array()) {
        return format!("enum:{values:?}");
    }
    if let Some(value) = schema.get("const") {
        return format!("const:{value}");
    }
    "any".into()
}

fn annotate(schema: &Value, mut text: String) -> String {
    if let Some(format) = schema.get("format").and_then(|f| f.as_str()) {
        text = format!("{text}/{format}");
    }
    if let Some(min) = schema.get("minimum") {
        text = format!("{text}>={min}");
    }
    text
}

fn concrete(schema: &Value, root: &Value, seen: &mut Vec<String>) -> Value {
    let schema = if let Some(reference) = schema.get("$ref").and_then(|r| r.as_str()) {
        if seen.iter().any(|item| item == reference) {
            return serde_json::json!({ "description": "recursive" });
        }
        let name = reference.rsplit('/').next().unwrap_or(reference);
        let Some(def) = root
            .get("$defs")
            .or_else(|| root.get("definitions"))
            .and_then(|defs| defs.get(name))
        else {
            return schema.clone();
        };
        seen.push(reference.to_owned());
        let resolved = concrete(def, root, seen);
        seen.pop();
        resolved
    } else {
        schema.clone()
    };
    let Some(parts) = schema.get("allOf").and_then(|p| p.as_array()) else {
        return schema;
    };
    let mut merged = strip_key(&schema, "allOf");
    for part in parts {
        merged = merge_objects(&merged, &concrete(part, root, seen));
    }
    merged
}

fn strip_key(schema: &Value, key: &str) -> Value {
    let mut kept = schema.clone();
    if let Some(obj) = kept.as_object_mut() {
        obj.remove(key);
    }
    kept
}

fn merge_objects(left: &Value, right: &Value) -> Value {
    let (Some(a), Some(b)) = (left.as_object(), right.as_object()) else {
        return right.clone();
    };
    let mut out = a.clone();
    for (key, value) in b {
        match key.as_str() {
            "properties" => {
                let slot = out
                    .entry(key.clone())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !slot.is_object() {
                    *slot = Value::Object(serde_json::Map::new());
                }
                if let Some(extra) = value.as_object() {
                    let obj = slot.as_object_mut().expect("properties");
                    for (name, schema) in extra {
                        obj.insert(name.clone(), schema.clone());
                    }
                }
            }
            "required" => {
                let slot = out
                    .entry(key.clone())
                    .or_insert_with(|| Value::Array(vec![]));
                if !slot.is_array() {
                    *slot = Value::Array(vec![]);
                }
                if let Some(items) = value.as_array() {
                    let list = slot.as_array_mut().expect("required");
                    for item in items {
                        if !list.contains(item) {
                            list.push(item.clone());
                        }
                    }
                }
            }
            "oneOf" | "anyOf" => {
                let mut list = out
                    .get(key)
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                if let Some(extra) = value.as_array() {
                    list.extend(extra.iter().cloned());
                }
                out.insert(key.clone(), Value::Array(list));
            }
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(out)
}

fn walk(
    direction: &str,
    version: &str,
    path: &str,
    old: &Node,
    new: &Node,
    breaks: &mut Vec<Break>,
) {
    match (old, new) {
        (Node::Union(before), Node::Union(after)) => {
            for (name, old_body) in before {
                let next = join(path, name);
                match after.iter().find(|(kept, _)| kept == name) {
                    None => breaks.push(removed_variant(direction, version, &next, name)),
                    Some((_, new_body)) => {
                        walk(direction, version, &next, old_body, new_body, breaks)
                    }
                }
            }
        }
        (Node::Object(before), Node::Object(after)) => {
            compare_fields(direction, version, path, before, after, breaks)
        }
        (Node::Array(before), Node::Array(after)) => walk(
            direction,
            version,
            &format!("{path}[]"),
            before,
            after,
            breaks,
        ),
        (Node::Map(before), Node::Map(after)) => walk(
            direction,
            version,
            &format!("{path}{{}}"),
            before,
            after,
            breaks,
        ),
        (Node::Leaf(before), Node::Leaf(after)) if before == after => {}
        // Inbound, a new alternative is fine when one of them still accepts what the old app sends.
        (old_node, Node::Union(after))
            if direction == "inbound" && !matches!(old_node, Node::Union(_)) =>
        {
            let mut best: Option<Vec<Break>> = None;
            for (_, body) in after {
                let mut nested = vec![];
                walk(direction, version, path, old_node, body, &mut nested);
                if nested.is_empty() {
                    return;
                }
                if best.as_ref().is_none_or(|kept| nested.len() < kept.len()) {
                    best = Some(nested);
                }
            }
            if let Some(nested) = best {
                breaks.extend(nested);
            }
        }
        _ => breaks.push(type_change(direction, version, path, old, new)),
    }
}

fn compare_fields(
    direction: &str,
    version: &str,
    path: &str,
    old: &BTreeMap<String, Field>,
    new: &BTreeMap<String, Field>,
    breaks: &mut Vec<Break>,
) {
    if direction == "inbound" {
        for (name, new_field) in new {
            let next = join(path, name);
            match old.get(name) {
                None if new_field.required => breaks.push(field_break(
                    direction,
                    version,
                    &next,
                    "required_field",
                    &format!("apps {version} omit `{next}`; this hub requires it"),
                )),
                None => {}
                Some(old_field) => {
                    if new_field.required && !old_field.required {
                        breaks.push(field_break(
                            direction,
                            version,
                            &next,
                            "required_field",
                            &format!("apps {version} omit `{next}`; this hub requires it"),
                        ));
                    }
                    walk(
                        direction,
                        version,
                        &next,
                        &old_field.node,
                        &new_field.node,
                        breaks,
                    );
                }
            }
        }
        return;
    }
    for (name, old_field) in old {
        let next = join(path, name);
        match new.get(name) {
            None if old_field.required => breaks.push(field_break(
                direction,
                version,
                &next,
                "removed_field",
                &format!("apps {version} read `{next}`; this hub no longer sends it"),
            )),
            None => {}
            Some(new_field) => {
                if old_field.required && !new_field.required {
                    breaks.push(field_break(
                        direction,
                        version,
                        &next,
                        "now_optional",
                        &format!(
                            "apps {version} read `{next}`; this hub no longer always sends it"
                        ),
                    ));
                }
                walk(
                    direction,
                    version,
                    &next,
                    &old_field.node,
                    &new_field.node,
                    breaks,
                );
            }
        }
    }
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.into()
    } else {
        format!("{path}.{name}")
    }
}

fn removed_variant(direction: &str, version: &str, path: &str, name: &str) -> Break {
    let impact = if direction == "inbound" {
        format!("apps {version} send `{name}`; this hub rejects it")
    } else {
        format!("apps {version} read `{name}`; this hub no longer sends it")
    };
    Break {
        version: version.into(),
        direction: direction.into(),
        path: path.into(),
        kind: "removed_variant".into(),
        impact,
    }
}

fn field_break(direction: &str, version: &str, path: &str, kind: &str, impact: &str) -> Break {
    Break {
        version: version.into(),
        direction: direction.into(),
        path: path.into(),
        kind: kind.into(),
        impact: impact.into(),
    }
}

fn type_change(direction: &str, version: &str, path: &str, old: &Node, new: &Node) -> Break {
    let impact = if direction == "inbound" {
        format!(
            "apps {version} send `{path}` as {}; this hub reads it as {}",
            describe(old),
            describe(new)
        )
    } else {
        format!(
            "apps {version} read `{path}` as {}; this hub sends {}",
            describe(old),
            describe(new)
        )
    };
    Break {
        version: version.into(),
        direction: direction.into(),
        path: path.into(),
        kind: "type_change".into(),
        impact,
    }
}

fn describe(node: &Node) -> String {
    match node {
        Node::Leaf(text) => text.clone(),
        Node::Object(_) => "object".into(),
        Node::Union(variants) => format!(
            "union({})",
            variants
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ),
        Node::Array(_) => "array".into(),
        Node::Map(_) => "map".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Usage;
    use crate::connectors::RunsOn;
    use crate::wire::{HubCall, ToMachine};
    use crate::workspace::PrAction;

    fn same(schema: Value) -> Schemas {
        Schemas {
            send: schema.clone(),
            accept: schema,
        }
    }

    fn contract(
        version: &str,
        inbound: Vec<(&str, Value)>,
        outbound: Vec<(&str, Value)>,
    ) -> Contract {
        Contract {
            version: version.into(),
            routes: vec!["POST /api/login".into(), "GET /ws/client".into()],
            inbound: inbound
                .into_iter()
                .map(|(k, v)| (k.into(), same(v)))
                .collect(),
            outbound: outbound
                .into_iter()
                .map(|(k, v)| (k.into(), same(v)))
                .collect(),
        }
    }

    fn outbound(version: &str, name: &str, schemas: Schemas) -> Contract {
        let mut outbound = BTreeMap::new();
        outbound.insert(name.into(), schemas);
        Contract {
            version: version.into(),
            routes: vec!["POST /api/login".into(), "GET /ws/client".into()],
            inbound: BTreeMap::new(),
            outbound,
        }
    }

    fn remove_call_field(schema: &mut Value, cmd: &str, field: &str) {
        let branches = schema["$defs"]["Call"]["oneOf"]
            .as_array_mut()
            .expect("Call oneOf");
        let branch = branches
            .iter_mut()
            .find(|branch| branch["properties"]["cmd"]["const"].as_str() == Some(cmd))
            .unwrap_or_else(|| panic!("{cmd}"));
        branch["properties"].as_object_mut().unwrap().remove(field);
        if let Some(required) = branch
            .get_mut("required")
            .and_then(|value| value.as_array_mut())
        {
            required.retain(|item| item.as_str() != Some(field));
        }
    }

    fn drop_variant(schema: &mut Value, name: &str) {
        let branches = schema
            .get_mut("oneOf")
            .and_then(|v| v.as_array_mut())
            .expect("oneOf");
        branches.retain(|branch| branch["properties"]["cmd"]["const"].as_str() != Some(name));
    }

    fn require(schema: &mut Value, variant: &str, field: &str) {
        let branches = schema
            .get_mut("oneOf")
            .and_then(|v| v.as_array_mut())
            .expect("oneOf");
        let branch = branches
            .iter_mut()
            .find(|branch| branch["properties"]["cmd"]["const"].as_str() == Some(variant))
            .expect(variant);
        let required = branch
            .as_object_mut()
            .unwrap()
            .entry("required")
            .or_insert_with(|| Value::Array(vec![]));
        let list = required.as_array_mut().unwrap();
        if !list.iter().any(|item| item.as_str() == Some(field)) {
            list.push(Value::String(field.into()));
        }
    }

    fn add_variant(schema: &mut Value, name: &str) {
        let branches = schema
            .get_mut("oneOf")
            .and_then(|v| v.as_array_mut())
            .expect("oneOf");
        branches.push(serde_json::json!({
            "type": "object",
            "properties": { "cmd": { "type": "string", "const": name } },
            "required": ["cmd"]
        }));
    }

    fn add_optional_field(schema: &mut Value, variant: &str, field: &str) {
        let branches = schema
            .get_mut("oneOf")
            .and_then(|v| v.as_array_mut())
            .expect("oneOf");
        let branch = branches
            .iter_mut()
            .find(|branch| branch["properties"]["cmd"]["const"].as_str() == Some(variant))
            .expect(variant);
        branch["properties"][field] = serde_json::json!({ "type": "string", "default": "" });
    }

    #[test]
    fn removing_a_hub_call_variant_is_one_inbound_break() {
        let full = schema_de::<HubCall>();
        let mut dropped = full.clone();
        drop_variant(&mut dropped, "connect_slack");
        let breaks = diff(
            &contract("0.8.4", vec![("HubCall", full)], vec![]),
            &contract("0.13.0", vec![("HubCall", dropped)], vec![]),
        );
        assert_eq!(
            breaks,
            vec![Break {
                version: "0.8.4".into(),
                direction: "inbound".into(),
                path: "HubCall.connect_slack".into(),
                kind: "removed_variant".into(),
                impact: "apps 0.8.4 send `connect_slack`; this hub rejects it".into(),
            }]
        );
    }

    #[test]
    fn making_an_inbound_field_required_is_a_break() {
        let full = schema_de::<HubCall>();
        let mut required = full.clone();
        require(&mut required, "import", "github_key");
        let breaks = diff(
            &contract("0.8.4", vec![("HubCall", full)], vec![]),
            &contract("0.13.0", vec![("HubCall", required)], vec![]),
        );
        assert_eq!(
            breaks,
            vec![Break {
                version: "0.8.4".into(),
                direction: "inbound".into(),
                path: "HubCall.import.github_key".into(),
                kind: "required_field".into(),
                impact: "apps 0.8.4 omit `HubCall.import.github_key`; this hub requires it".into(),
            }]
        );
    }

    #[test]
    fn a_new_variant_and_an_optional_field_are_not_breaks() {
        let full = schema_de::<HubCall>();
        let mut grown = full.clone();
        add_variant(&mut grown, "ping");
        add_optional_field(&mut grown, "connect_slack", "note");
        let breaks = diff(
            &contract("0.8.4", vec![("HubCall", full)], vec![]),
            &contract("0.13.0", vec![("HubCall", grown)], vec![]),
        );
        assert_eq!(breaks, vec![]);
    }

    #[test]
    fn removing_a_machine_view_field_is_an_outbound_break() {
        let full = schema_ser::<MachineView>();
        let mut dropped = full.clone();
        dropped["properties"]
            .as_object_mut()
            .unwrap()
            .remove("online");
        let required = dropped["required"].as_array_mut().unwrap();
        required.retain(|item| item.as_str() != Some("online"));
        let breaks = diff(
            &contract("0.8.4", vec![], vec![("MachineView", full)]),
            &contract("0.13.0", vec![], vec![("MachineView", dropped)]),
        );
        assert_eq!(
            breaks,
            vec![Break {
                version: "0.8.4".into(),
                direction: "outbound".into(),
                path: "MachineView.online".into(),
                kind: "removed_field".into(),
                impact: "apps 0.8.4 read `MachineView.online`; this hub no longer sends it".into(),
            }]
        );
    }

    #[test]
    fn property_order_does_not_rename_a_variant() {
        let full = schema_de::<Usage>();
        let mut reordered = full.clone();
        let window = reordered["$defs"]["LimitWindow"]["properties"]
            .as_object_mut()
            .unwrap();
        let mut pairs: Vec<_> = window
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        pairs.rotate_left(1);
        window.clear();
        for (key, value) in pairs {
            window.insert(key, value);
        }
        let breaks = diff(
            &contract("0.8.4", vec![("Usage", full)], vec![]),
            &contract("0.13.0", vec![("Usage", reordered)], vec![]),
        );
        assert_eq!(breaks, vec![]);
    }

    #[test]
    fn adding_an_enum_value_is_not_a_break() {
        let full = schema_de::<PrAction>();
        let mut older = full.clone();
        let branch = older["oneOf"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|branch| branch.get("enum").is_some())
            .unwrap();
        branch["enum"]
            .as_array_mut()
            .unwrap()
            .retain(|value| value.as_str() != Some("close"));
        let breaks = diff(
            &contract("0.8.4", vec![("PrAction", older)], vec![]),
            &contract("0.13.0", vec![("PrAction", full)], vec![]),
        );
        assert_eq!(breaks, vec![]);
    }

    #[test]
    fn a_string_the_hub_still_accepts_is_not_an_inbound_break() {
        let widened = schema_de::<RunsOn>();
        let string = serde_json::json!({ "type": "string" });
        let inbound = diff(
            &contract("0.8.4", vec![("RunsOn", string.clone())], vec![]),
            &contract("0.13.0", vec![("RunsOn", widened.clone())], vec![]),
        );
        assert_eq!(inbound, vec![]);
        let outbound = diff(
            &contract("0.8.4", vec![], vec![("RunsOn", string)]),
            &contract("0.13.0", vec![], vec![("RunsOn", widened)]),
        );
        assert_eq!(
            outbound,
            vec![Break {
                version: "0.8.4".into(),
                direction: "outbound".into(),
                path: "RunsOn".into(),
                kind: "type_change".into(),
                impact: "apps 0.8.4 read `RunsOn` as string; this hub sends union(Machine,Box)"
                    .into(),
            }]
        );
    }

    #[test]
    fn a_route_only_the_old_hub_served_is_a_break() {
        let mut old = contract("0.8.4", vec![], vec![]);
        old.routes = vec!["POST /api/login".into(), "GET /ws/client".into()];
        let mut new = old.clone();
        new.version = "0.13.0".into();
        new.routes.retain(|route| route != "GET /ws/client");
        let breaks = diff(&old, &new);
        assert_eq!(
            breaks,
            vec![Break {
                version: "0.8.4".into(),
                direction: "route".into(),
                path: "GET /ws/client".into(),
                kind: "removed_route".into(),
                impact: "apps 0.8.4 call `GET /ws/client`; this hub no longer serves it".into(),
            }]
        );
    }

    #[test]
    fn serde_reads_a_missing_option_without_a_default_attribute() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Row {
            kept: String,
            extra: Option<String>,
        }
        let row: Row = serde_json::from_str(r#"{"kept":"yes"}"#).unwrap();
        assert_eq!(
            row,
            Row {
                kept: "yes".into(),
                extra: None,
            }
        );
    }

    #[test]
    fn runs_on_sends_and_accepts_the_stored_union() {
        let wire = schemas::<RunsOn>();
        assert_eq!(
            describe(&node(&wire.send, &wire.send)),
            "union(Machine,Box)"
        );
        assert_eq!(
            describe(&node(&wire.accept, &wire.accept)),
            "union(Machine,Box)"
        );
    }

    #[test]
    fn the_hub_accepts_what_it_sends() {
        let hub = current("0.13.0", &["POST /api/login", "GET /ws/client"]);
        assert_eq!(diff(&hub, &hub), vec![]);
    }

    #[test]
    fn dropping_a_defaulted_outbound_field_is_not_a_break_and_dropping_a_required_one_is() {
        let old_wire = schemas::<ToMachine>();
        let mut without_prices = old_wire.send.clone();
        remove_call_field(&mut without_prices, "set_usage_prices", "prices");
        let quiet = diff(
            &outbound("0.12.0", "ToMachine", old_wire.clone()),
            &outbound(
                "0.13.0",
                "ToMachine",
                Schemas {
                    send: without_prices,
                    accept: old_wire.accept.clone(),
                },
            ),
        );
        assert_eq!(quiet, vec![]);

        let mut without_since = old_wire.send.clone();
        remove_call_field(&mut without_since, "token_usage", "since");
        let breaks = diff(
            &outbound("0.12.0", "ToMachine", old_wire.clone()),
            &outbound(
                "0.13.0",
                "ToMachine",
                Schemas {
                    send: without_since,
                    accept: old_wire.accept,
                },
            ),
        );
        assert_eq!(
            breaks,
            vec![Break {
                version: "0.12.0".into(),
                direction: "outbound".into(),
                path: "ToMachine.call.call.token_usage.since".into(),
                kind: "removed_field".into(),
                impact: "apps 0.12.0 read `ToMachine.call.call.token_usage.since`; this hub no longer sends it".into(),
            }]
        );
    }

    #[test]
    fn a_break_shared_with_the_base_is_not_new() {
        let string = serde_json::json!({ "type": "string" });
        let widened = schema_de::<RunsOn>();
        let snapshot = contract("0.8.4", vec![], vec![("RunsOn", string)]);
        let hub = contract("0.13.0", vec![], vec![("RunsOn", widened)]);
        let breaks = diff(&snapshot, &hub);
        let shipped = Break {
            version: "0.8.4".into(),
            direction: "outbound".into(),
            path: "RunsOn".into(),
            kind: "type_change".into(),
            impact: "apps 0.8.4 read `RunsOn` as string; this hub sends union(Machine,Box)".into(),
        };
        assert_eq!(breaks, vec![shipped.clone()]);
        let delta = classify(&breaks, &breaks, "");
        assert_eq!(delta.new_breaks, vec![]);
        assert_eq!(delta.shipped, vec![shipped]);
    }

    #[test]
    fn removing_a_hub_call_variant_is_the_only_new_break() {
        let full = schema_de::<HubCall>();
        let mut dropped = full.clone();
        drop_variant(&mut dropped, "connect_slack");
        let snapshot = contract("0.8.4", vec![("HubCall", full.clone())], vec![]);
        let base_hub = contract("base", vec![("HubCall", full)], vec![]);
        let head_hub = contract("head", vec![("HubCall", dropped)], vec![]);
        let base_breaks = diff(&snapshot, &base_hub);
        let head_breaks = diff(&snapshot, &head_hub);
        assert_eq!(base_breaks, vec![]);
        let found = Break {
            version: "0.8.4".into(),
            direction: "inbound".into(),
            path: "HubCall.connect_slack".into(),
            kind: "removed_variant".into(),
            impact: "apps 0.8.4 send `connect_slack`; this hub rejects it".into(),
        };
        assert_eq!(head_breaks, vec![found.clone()]);
        let delta = classify(&head_breaks, &base_breaks, "");
        assert_eq!(delta.new_breaks, vec![found]);
        assert_eq!(delta.shipped, vec![]);
    }

    #[test]
    fn deleting_a_snapshot_is_a_new_break() {
        let status = "\
M\tcrates/core/wire-contract/current.json
D\tcrates/core/wire-contract/0.9.0.json
A\tcrates/core/wire-contract/0.14.0.json
";
        let delta = classify(&[], &[], status);
        assert_eq!(
            delta.new_breaks,
            vec![Break {
                version: "0.9.0".into(),
                direction: "snapshot".into(),
                path: "crates/core/wire-contract/0.9.0.json".into(),
                kind: "deleted_snapshot".into(),
                impact: "Deleting `0.9.0` hides a version this hub must still speak.".into(),
            }]
        );
        assert_eq!(delta.shipped, vec![]);
        let edited = classify(&[], &[], "M\tcrates/core/wire-contract/0.12.0.json\n");
        assert_eq!(
            edited.new_breaks,
            vec![Break {
                version: "0.12.0".into(),
                direction: "snapshot".into(),
                path: "crates/core/wire-contract/0.12.0.json".into(),
                kind: "edited_snapshot".into(),
                impact: "Editing `0.12.0` changes the recorded wire of that version.".into(),
            }]
        );
    }

    #[test]
    fn a_changed_contract_is_stale() {
        let generated = Contract {
            version: CURRENT_VERSION.into(),
            routes: vec!["POST /api/login".into()],
            inbound: BTreeMap::new(),
            outbound: BTreeMap::new(),
        };
        let mut committed = generated.clone();
        committed.routes.push("GET /ws/client".into());
        assert_eq!(
            require_fresh(&generated, Some(&committed)).unwrap_err(),
            STALE
        );
        assert_eq!(require_fresh(&generated, None).unwrap_err(), STALE);
        assert!(require_fresh(&generated, Some(&generated)).is_ok());
    }
}
