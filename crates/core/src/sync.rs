//! A project's Claude memory and sessions, its skills git doesn't carry, and pinned's history of
//! its turns, kept the same on every machine of the account through the hub.
//!
//! A project is named by its git origin, since each machine keeps it in a folder of its own. Every
//! machine compares its files with the hub's and with what both held at its last sync, so a file
//! changed on one side only travels that way, a file deleted on one side is deleted on the other,
//! and a file changed on both keeps the newer copy; the history keeps the turns of both.

use crate::process;
use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A session untouched this long is never sent the first time; one already shared keeps syncing.
const SESSION_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
/// Files bigger than this, compressed, stay on their machine.
pub const FILE_LIMIT: u64 = 32 << 20;
/// Compressed bytes sent or fetched in one call.
const BATCH: u64 = 8 << 20;
/// The biggest websocket message a sync call makes: a batch just short of `BATCH` plus one file
/// at `FILE_LIMIT`, in base64, with room for the JSON around it. Both ends of a machine's socket
/// take messages this big, well past tungstenite's 16 MB default frame.
pub const MESSAGE_LIMIT: usize = ((BATCH + FILE_LIMIT) as usize).div_ceil(3) * 4 + (1 << 20);
pub const HISTORY: &str = "history.jsonl";

/// A file as the hub holds it. An empty `sha` marks a deleted one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Entry {
    pub path: String,
    pub sha: String,
    /// Unix milliseconds the file last changed on the machine that sent it.
    pub modified: u64,
    /// Compressed bytes.
    #[serde(default)]
    pub size: u64,
    /// Unix mode when the sender read one. Absent when the sender does not know the mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct File {
    #[serde(flatten)]
    pub entry: Entry,
    /// The contents gzipped, in base64; none for a deleted file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// Sets the executable bit on a file the hub already stores, without sending bytes.
/// The hub applies it only while that file's sha is still `sha`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct ModeRepair {
    pub path: String,
    pub sha: String,
}

/// A project of the account, offered on machines that don't have it yet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Project {
    /// The origin as `host/owner/name`, lowercased.
    pub key: String,
    pub name: String,
    /// The origin as the machine that shared it has it, which clones it elsewhere.
    pub repo: String,
}

/// A project as it sits on this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkout {
    pub project: Project,
    pub path: PathBuf,
    /// Where pinned keeps this checkout's turns.
    pub history: Option<PathBuf>,
    /// The Claude Code home of the account the project runs under, which holds its sessions and
    /// memory. None for a project on another provider, which shares only its skills and turns.
    pub claude_home: Option<PathBuf>,
}

/// What a machine remembers between syncs: the files' hashes, to skip hashing unchanged files,
/// and what each checkout and the hub agreed on last.
#[derive(Serialize, Deserialize, Default, Debug, schemars::JsonSchema)]
pub struct State {
    #[serde(default)]
    hashes: BTreeMap<PathBuf, Hashed>,
    /// By `<key> <checkout path>`, so a project moved to another folder starts over instead of
    /// taking its files for deleted.
    #[serde(default)]
    agreed: BTreeMap<String, BTreeMap<String, String>>,
    /// The Claude home each checkout's agreed `claude/` files were in, by the same key. A project
    /// moved to another account starts its Claude files over instead of taking them for deleted.
    #[serde(default)]
    claude_homes: BTreeMap<String, PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, schemars::JsonSchema)]
struct Hashed {
    size: u64,
    modified: u64,
    sha: String,
}

impl State {
    /// A state saved before accounts names no Claude home; its files were in `legacy_claude_home`.
    pub fn load(file: &Path, legacy_claude_home: &Path) -> State {
        let mut state: State = std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        for key in state.agreed.keys() {
            if key.starts_with(PLUGINS_KEY) {
                continue;
            }
            state.claude_homes.entry(key.clone()).or_insert_with(|| legacy_claude_home.to_owned());
        }
        state
    }

    pub fn save(&self, file: &Path) {
        let _ = std::fs::write(file, serde_json::to_vec(self).unwrap());
    }

    fn hash(&mut self, file: &Path) -> Option<(String, u64)> {
        let meta = std::fs::metadata(file).ok().filter(|m| m.is_file())?;
        let modified = millis(meta.modified().ok()?);
        if let Some(h) = self.hashes.get(file).filter(|h| h.size == meta.len() && h.modified == modified) {
            return Some((h.sha.clone(), modified));
        }
        let sha = sha(&std::fs::read(file).ok()?);
        self.hashes.insert(file.to_owned(), Hashed { size: meta.len(), modified, sha: sha.clone() });
        Some((sha, modified))
    }
}

/// What a sync of one checkout does through the hub.
pub trait Hub {
    fn list(&self, project: &str) -> Result<Vec<Entry>, String>;
    fn push(&self, project: &str, files: Vec<File>) -> Result<(), String>;
    fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<File>, String>;
    fn mark_executable(&self, project: &str, repairs: Vec<ModeRepair>) -> Result<(), String>;
}

#[derive(Debug, Default, PartialEq)]
pub struct Synced {
    pub pushed: Vec<String>,
    pub pulled: Vec<String>,
}

/// Claude Code's folder for a project's sessions and memory in `claude_home`: the path with every
/// character but letters and digits made a dash.
pub fn claude_dir(claude_home: &Path, checkout: &Path) -> PathBuf {
    let slug: String = checkout.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    claude_home.join("projects").join(slug)
}

/// Brings one checkout and the hub to the same files.
pub fn sync(hub: &impl Hub, state: &mut State, checkout: &Checkout, now: SystemTime) -> Result<Synced, String> {
    let roots = Roots::new(checkout);
    let key = &checkout.project.key;
    let mut local: BTreeMap<String, (String, u64)> = BTreeMap::new();
    for (path, file) in roots.files() {
        if let Some(hashed) = state.hash(&file) {
            local.insert(path, hashed);
        }
    }
    let agreed_key = format!("{key} {}", checkout.path.display());
    let mut agreed = state.agreed.remove(&agreed_key).unwrap_or_default();
    if let Some(home) = &checkout.claude_home {
        if state.claude_homes.insert(agreed_key.clone(), home.clone()).is_some_and(|before| &before != home) {
            agreed.retain(|path, _| !path.starts_with("claude/"));
        }
    }
    let synced = exchange(hub, state, key, &mut agreed, &local, &|path| roots.local(path), now, true)?;
    state.agreed.insert(agreed_key, agreed);
    Ok(synced)
}

fn exchange(
    hub: &impl Hub,
    state: &mut State,
    key: &str,
    mut agreed: &mut BTreeMap<String, String>,
    local: &BTreeMap<String, (String, u64)>,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
    now: SystemTime,
    sessions: bool,
) -> Result<Synced, String> {
    let remote: BTreeMap<String, Entry> = hub.list(key)?.into_iter().map(|e| (e.path.clone(), e)).collect();
    // A file of a folder this checkout doesn't share, such as Claude's for a Codex project, is
    // neither fetched nor taken for deleted.
    let paths: BTreeSet<String> = local.keys().chain(remote.keys()).chain(agreed.keys()).filter(|p| resolve(p).is_some()).cloned().collect();
    let (mut push, mut pull, mut merge) = (vec![], vec![], vec![]);
    #[cfg(unix)]
    let mut repairs: Vec<ModeRepair> = Vec::new();
    for path in &paths {
        let mine = local.get(path).map(|(sha, _)| sha.clone());
        let theirs = remote.get(path).map(|e| e.sha.clone()).filter(|sha| !sha.is_empty());
        let before = agreed.get(path).cloned();
        if mine == theirs {
            match mine {
                Some(sha) => {
                    agreed.insert(path.clone(), sha.clone());
                    #[cfg(not(unix))]
                    drop(sha);
                    #[cfg(unix)]
                    if key == PLUGINS_KEY {
                        if let Some(fix) = unix_mode_fix(path, &remote, resolve)? {
                            match fix {
                                ModeFix::Mark => repairs.push(ModeRepair { path: path.clone(), sha }),
                                ModeFix::Chmod => {
                                    if let Some(target) = resolve(path) {
                                        set_owner_execute(&target)?;
                                    }
                                }
                            }
                        }
                    }
                }
                None => {
                    agreed.remove(path);
                }
            };
        } else if mine == before {
            pull.push(path.clone());
        } else if theirs == before {
            let old = sessions && local.get(path).is_some_and(|(_, modified)| millis(now).saturating_sub(*modified) > SESSION_AGE.as_millis() as u64);
            if !(before.is_none() && theirs.is_none() && old && path.starts_with("claude/") && path.ends_with(".jsonl")) {
                push.push(path.clone());
            }
        } else if sessions && path.as_str() == HISTORY {
            merge.push(path.clone());
        } else if local.get(path).map_or(0, |(_, m)| *m) >= remote.get(path).map_or(0, |e| e.modified) {
            push.push(path.clone());
        } else {
            pull.push(path.clone());
        }
    }
    #[cfg(unix)]
    if !repairs.is_empty() {
        // A content push would write these bytes and their mtime over whatever landed since list.
        hub.mark_executable(key, repairs)?;
    }
    let mut synced = Synced::default();
    for path in merge {
        let theirs = hub.pull(key, vec![path.clone()])?.into_iter().next().and_then(|f| f.data).map(|d| unpack(&d)).transpose()?.unwrap_or_default();
        let target = resolve(&path).ok_or("no history here")?;
        let mine = std::fs::read(&target).unwrap_or_default();
        write(&target, &merge_history(&mine, &theirs), None, None)?;
        push.push(path);
    }

    let mut batch: Vec<File> = vec![];
    let mut size = 0;
    for path in push {
        let file = match resolve(&path).filter(|f| f.is_file()) {
            None => File { entry: Entry { path: path.clone(), sha: String::new(), modified: millis(now), size: 0, executable: None }, data: None },
            Some(target) => {
                let Some((sha, modified)) = state.hash(&target) else { continue };
                let data = pack(&std::fs::read(&target).map_err(|e| e.to_string())?);
                if data.len() as u64 > FILE_LIMIT {
                    continue;
                }
                File {
                    entry: Entry { path: path.clone(), sha, modified, size: data.len() as u64, executable: if key == PLUGINS_KEY { executable(&target)? } else { None } },
                    data: Some(base64::engine::general_purpose::STANDARD.encode(data)),
                }
            }
        };
        size += file.entry.size;
        match &file.data {
            Some(_) => agreed.insert(path.clone(), file.entry.sha.clone()),
            None => agreed.remove(&path),
        };
        batch.push(file);
        synced.pushed.push(path);
        if size >= BATCH {
            hub.push(key, std::mem::take(&mut batch))?;
            size = 0;
        }
    }
    if !batch.is_empty() {
        hub.push(key, batch)?;
    }

    let mut wanted: Vec<String> = vec![];
    let mut size = 0;
    let fetch = |wanted: Vec<String>, agreed: &mut BTreeMap<String, String>, synced: &mut Synced, state: &mut State| -> Result<(), String> {
        for file in hub.pull(key, wanted)? {
            let Some(target) = resolve(&file.entry.path) else { continue };
            match &file.data {
                None => {
                    let _ = std::fs::remove_file(&target);
                    agreed.remove(&file.entry.path);
                }
                Some(data) => {
                    let bytes = unpack(data)?;
                    let mode = if key == PLUGINS_KEY { file.entry.executable } else { None };
                    write(&target, &bytes, Some(file.entry.modified), mode)?;
                    state.hash(&target);
                    agreed.insert(file.entry.path.clone(), file.entry.sha.clone());
                }
            }
            synced.pulled.push(file.entry.path);
        }
        Ok(())
    };
    for path in pull {
        if !remote.contains_key(&path) {
            // Deleted here and never on the hub: nothing to fetch, and nothing agreed anymore.
            agreed.remove(&path);
            continue;
        }
        size += remote[&path].size;
        wanted.push(path);
        if size >= BATCH {
            fetch(std::mem::take(&mut wanted), &mut agreed, &mut synced, state)?;
            size = 0;
        }
    }
    if !wanted.is_empty() {
        fetch(wanted, &mut agreed, &mut synced, state)?;
    }
    Ok(synced)
}

/// The folders a checkout shares, and how their files are named on the hub.
struct Roots {
    claude: Option<PathBuf>,
    /// None while git tracks the skills, which then travel with the repository.
    skills: Option<PathBuf>,
    history: Option<PathBuf>,
}

impl Roots {
    fn new(checkout: &Checkout) -> Roots {
        let skills = checkout.path.join(".claude/skills");
        Roots {
            claude: checkout.claude_home.as_ref().map(|home| claude_dir(home, &checkout.path)),
            skills: (!tracked(&checkout.path, ".claude/skills")).then_some(skills),
            history: checkout.history.clone(),
        }
    }

    /// Every shared file here, by its name on the hub.
    fn files(&self) -> Vec<(String, PathBuf)> {
        let mut files = vec![];
        if let Some(claude) = &self.claude {
            for entry in std::fs::read_dir(claude).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".jsonl") && entry.path().is_file() {
                    files.push((format!("claude/{name}"), entry.path()));
                }
            }
            walk(&claude.join("memory"), "claude/memory", &mut files);
        }
        if let Some(skills) = &self.skills {
            walk(skills, "skills", &mut files);
        }
        if let Some(history) = self.history.as_ref().filter(|h| h.is_file()) {
            files.push((HISTORY.into(), history.clone()));
        }
        files
    }

    /// Where a file named `path` on the hub goes here, or None when it names no shared folder.
    fn local(&self, path: &str) -> Option<PathBuf> {
        if path == HISTORY {
            return self.history.clone();
        }
        let (root, rest, claude) = if let Some(rest) = path.strip_prefix("claude/") {
            (self.claude.as_ref()?, rest, true)
        } else if let Some(rest) = path.strip_prefix("skills/") {
            (self.skills.as_ref()?, rest, false)
        } else {
            return None;
        };
        let rest = Path::new(rest);
        let inside = rest.components().all(|c| matches!(c, Component::Normal(_))) && rest.components().next().is_some();
        let shared = rest.starts_with("memory") || (rest.components().count() == 1 && rest.extension().is_some_and(|e| e == "jsonl"));
        (inside && (!claude || shared)).then(|| root.join(rest))
    }
}

/// Files of enabled plugins and of `~/.claude/skills`, one set per account.
pub const PLUGINS_KEY: &str = "pinned:plugins";

/// The Claude home that publishes plugins, and the pinned-owned folder pulled copies land in.
#[derive(Clone, Debug)]
pub struct PluginShare {
    pub claude_home: PathBuf,
    pub dir: PathBuf,
}

pub fn old_hub(err: &str) -> bool {
    err.contains("is not a file a project shares")
}

/// Folders a session loads with `--plugin-dir`. One per synced plugin this Claude home does not
/// already enable.
pub fn plugin_dirs(claude_home: &Path, dir: &Path) -> Vec<PathBuf> {
    let enabled: BTreeSet<String> = crate::commands::enabled_plugins(claude_home).into_iter().map(|plugin| plugin.id).collect();
    let mut dirs = vec![];
    let Ok(entries) = std::fs::read_dir(dir.join("plugins")) else { return dirs };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else { continue };
        if !path.is_dir() || enabled.contains(&id) || !path.join(".claude-plugin/plugin.json").is_file() {
            continue;
        }
        dirs.push(path);
    }
    dirs.sort();
    dirs
}

/// Brings this Claude home's enabled plugins and its `skills` folder into line with the hub.
/// Pulled plugins land under `share.dir`. Pulled user skills land in the home's `skills` folder.
/// Claude Code has no flag for a second skills directory.
/// A user skill this home already has, and has not synced before, stays when the hub holds a different copy.
pub fn sync_plugins(hub: &impl Hub, state: &mut State, share: &PluginShare, now: SystemTime) -> Result<Synced, String> {
    publish(share)?;
    let mut local = BTreeMap::new();
    for (path, file) in plugin_files(share) {
        if let Some(hashed) = state.hash(&file) {
            local.insert(path, hashed);
        }
    }
    let agreed_key = format!("{PLUGINS_KEY} {}", share.claude_home.display());
    let mut agreed = state.agreed.get(&agreed_key).cloned().unwrap_or_default();
    let keep = conflicting_user_skills(&local, &agreed, &hub.list(PLUGINS_KEY)?);
    let synced = exchange(hub, state, PLUGINS_KEY, &mut agreed, &local, &|path| if keep.contains(path) { None } else { plugin_local(share, path) }, now, false)?;
    state.agreed.insert(agreed_key, agreed);
    Ok(synced)
}

fn conflicting_user_skills(local: &BTreeMap<String, (String, u64)>, agreed: &BTreeMap<String, String>, remote: &[Entry]) -> BTreeSet<String> {
    let on_hub: BTreeMap<&str, &str> = remote.iter().map(|entry| (entry.path.as_str(), entry.sha.as_str())).collect();
    local
        .iter()
        .filter(|(path, (sha, _))| path.starts_with("user-skills/") && !agreed.contains_key(path.as_str()) && on_hub.get(path.as_str()).is_some_and(|hub_sha| *hub_sha != sha.as_str()))
        .map(|(path, _)| (*path).clone())
        .collect()
}

const DENY_FILE: &[&str] = &["settings.json", "settings.local.json", ".claude.json", "installed_plugins.json", ".env", "credentials.json", "id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"];
const SKIP_DIR: &[&str] = &["node_modules", ".git", "secrets", ".secrets", "target", "dist", "build", "vendor"];

fn blocked(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    DENY_FILE.iter().any(|item| *item == name) || SKIP_DIR.iter().any(|item| *item == name) || name.starts_with(".env") || name.ends_with(".pem") || name.ends_with(".key")
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// Whether the hub should store `path` for `project`. Plugin paths are stored only under `PLUGINS_KEY`.
pub fn accepts(project: &str, path: &str) -> bool {
    if project == PLUGINS_KEY { plugin_path(path) } else { shared_path(path) }
}

fn plugin_path(path: &str) -> bool {
    let rest = Path::new(path);
    if path.is_empty() || !rest.components().all(|c| matches!(c, Component::Normal(_))) {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|part| part.is_empty() || blocked(part)) {
        return false;
    }
    match parts.as_slice() {
        ["plugins", id, _, ..] if plugin_id(id) => true,
        ["user-skills", first, ..] if !first.eq_ignore_ascii_case("synced") && plugin_id(first) => true,
        _ => false,
    }
}

fn plugin_id(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains('/') && !id.contains('\\')
}

fn publish(share: &PluginShare) -> Result<(), String> {
    let record = share.dir.join(".published").join(sha(share.claude_home.to_string_lossy().as_bytes()));
    let before = read_ids(&record);
    let mut keep = BTreeSet::new();
    for plugin in crate::commands::enabled_plugins(&share.claude_home) {
        if !plugin_id(&plugin.id) || plugin.path.components().any(|c| c.as_os_str() == "marketplaces") || is_symlink(&plugin.path) {
            continue;
        }
        mirror(&plugin, &share.dir.join("plugins").join(&plugin.id))?;
        keep.insert(plugin.id);
    }
    for id in &before {
        if !keep.contains(id) && plugin_id(id) {
            let _ = std::fs::remove_dir_all(share.dir.join("plugins").join(id));
        }
    }
    if let Some(parent) = record.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(record, serde_json::to_vec(&keep).unwrap()).map_err(|e| e.to_string())
}

fn read_ids(file: &Path) -> BTreeSet<String> {
    std::fs::read(file).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn mirror(plugin: &crate::commands::EnabledPlugin, dest: &Path) -> Result<(), String> {
    if is_symlink(dest) {
        return Ok(());
    }
    copy_tree(&plugin.path, dest, &plugin.id, Path::new(""))?;
    let marker = dest.join(".pinned-plugin.json");
    let body = serde_json::to_vec(&serde_json::json!({ "version": plugin.version, "gitCommitSha": plugin.git_commit_sha })).unwrap();
    if std::fs::read(&marker).ok().as_deref() == Some(body.as_slice()) {
        return Ok(());
    }
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(marker, body).map_err(|e| e.to_string())
}

fn copy_tree(src: &Path, dest: &Path, id: &str, rel: &Path) -> Result<(), String> {
    if is_symlink(src) || is_symlink(dest) {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(src) else { return Ok(()) };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        if blocked(&name) {
            continue;
        }
        let rel = rel.join(&name);
        let src_path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&src_path) else { continue };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            copy_tree(&src_path, dest, id, &rel)?;
        } else if meta.is_file() {
            let Some(hub_path) = hub_join(&format!("plugins/{id}"), &rel) else { continue };
            if accepts(PLUGINS_KEY, &hub_path) {
                copy_newer(&src_path, &dest.join(&rel), &meta)?;
            }
        }
    }
    Ok(())
}

fn copy_newer(src: &Path, dest: &Path, meta: &std::fs::Metadata) -> Result<(), String> {
    if is_symlink(dest) {
        return Ok(());
    }
    let Some(have) = std::fs::metadata(dest).ok() else {
        return copy_file(src, dest, meta);
    };
    let src_exec = is_exec(meta);
    let dest_exec = is_exec(&have);
    match (meta.modified().ok(), have.modified().ok()) {
        (Some(src_mod), Some(dest_mod)) if src_mod > dest_mod => {
            if same_contents(src, dest, have.len()) {
                if src_exec && !dest_exec {
                    copy_mode(dest, meta)?;
                }
                let _ = std::fs::File::options().write(true).open(dest).and_then(|file| file.set_modified(src_mod));
            } else {
                copy_file(src, dest, meta)?;
            }
        }
        // chmod does not bump mtime. Same bytes stay executable once any copy is.
        (Some(_), Some(_)) if src_exec && !dest_exec && same_contents(src, dest, have.len()) => copy_mode(dest, meta)?,
        (Some(_), Some(_)) => {}
        _ if meta.len() != have.len() => copy_file(src, dest, meta)?,
        _ => {}
    }
    Ok(())
}

fn copy_file(src: &Path, dest: &Path, meta: &std::fs::Metadata) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::copy(src, dest).map_err(|e| e.to_string())?;
    if let Ok(modified) = meta.modified() {
        let _ = std::fs::File::options().write(true).open(dest).and_then(|file| file.set_modified(modified));
    }
    Ok(())
}

fn copy_mode(dest: &Path, meta: &std::fs::Metadata) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        let _ = (dest, meta);
    }
    Ok(())
}

fn is_exec(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        false
    }
}

fn same_contents(src: &Path, dest: &Path, dest_len: u64) -> bool {
    std::fs::metadata(src).is_ok_and(|meta| meta.len() == dest_len) && std::fs::read(src).ok() == std::fs::read(dest).ok()
}

fn plugin_files(share: &PluginShare) -> Vec<(String, PathBuf)> {
    let mut files = vec![];
    walk_shared(&share.dir.join("plugins"), "plugins", false, &mut files);
    walk_shared(&share.claude_home.join("skills"), "user-skills", true, &mut files);
    files
}

fn walk_shared(dir: &Path, prefix: &str, skills: bool, files: &mut Vec<(String, PathBuf)>) {
    if is_symlink(dir) {
        return;
    }
    fn rec(dir: &Path, prefix: &str, rel: &Path, skills: bool, files: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
            if blocked(&name) {
                continue;
            }
            if skills && rel.as_os_str().is_empty() && name.eq_ignore_ascii_case("synced") {
                continue;
            }
            let rel = rel.join(&name);
            let Some(hub_path) = hub_join(prefix, &rel) else { continue };
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                rec(&path, prefix, &rel, skills, files);
            } else if meta.is_file() && accepts(PLUGINS_KEY, &hub_path) {
                files.push((hub_path, path));
            }
        }
    }
    rec(dir, prefix, Path::new(""), skills, files);
}

fn hub_join(prefix: &str, rel: &Path) -> Option<String> {
    let mut parts = vec![prefix.to_owned()];
    for component in rel.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

fn plugin_local(share: &PluginShare, path: &str) -> Option<PathBuf> {
    if !accepts(PLUGINS_KEY, path) {
        return None;
    }
    if let Some(rest) = path.strip_prefix("plugins/") {
        return placed(&share.dir.join("plugins"), rest);
    }
    placed(&share.claude_home.join("skills"), path.strip_prefix("user-skills/")?)
}

fn placed(root: &Path, rest: &str) -> Option<PathBuf> {
    let rel = rel_path(rest)?;
    if is_symlink(root) {
        return None;
    }
    let mut cur = root.to_path_buf();
    for component in rel.components() {
        cur.push(component);
        if is_symlink(&cur) {
            return None;
        }
    }
    Some(cur)
}

fn rel_path(rest: &str) -> Option<PathBuf> {
    let rel = Path::new(rest);
    if rest.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(rel.to_path_buf())
}

/// Whether `path` names a file a project shares, before any machine maps it to a folder.
pub fn shared_path(path: &str) -> bool {
    let rest = Path::new(path);
    let inside = rest.components().all(|c| matches!(c, Component::Normal(_)));
    inside && (path == HISTORY || path.starts_with("claude/") || path.starts_with("skills/"))
}

fn walk(dir: &Path, prefix: &str, files: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if path.is_dir() {
            walk(&path, &format!("{prefix}/{name}"), files);
        } else if path.is_file() {
            files.push((format!("{prefix}/{name}"), path));
        }
    }
}

fn tracked(repo: &Path, path: &str) -> bool {
    let out = process::command("git").arg("-C").arg(repo).args(["ls-files", "--", path]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    out.is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// Both histories' turns, once each, oldest first as they were appended.
fn merge_history(mine: &[u8], theirs: &[u8]) -> Vec<u8> {
    let (mine, theirs) = (String::from_utf8_lossy(mine), String::from_utf8_lossy(theirs));
    let mut seen = BTreeSet::new();
    let mut lines: Vec<(u64, &str)> = mine
        .lines()
        .chain(theirs.lines())
        .filter(|l| !l.trim().is_empty() && seen.insert(*l))
        .map(|l| (serde_json::from_str::<serde_json::Value>(l).ok().and_then(|v| v["ended"].as_u64()).unwrap_or(0), l))
        .collect();
    lines.sort_by_key(|(ended, _)| *ended);
    lines.into_iter().flat_map(|(_, l)| [l.as_bytes(), b"\n"]).flatten().copied().collect()
}

#[cfg(unix)]
fn mode_fix(local: Option<bool>, remote: Option<bool>) -> Option<ModeFix> {
    match (local, remote) {
        (Some(true), Some(false)) => Some(ModeFix::Mark),
        (Some(false), Some(true)) => Some(ModeFix::Chmod),
        _ => None,
    }
}

#[cfg(unix)]
enum ModeFix {
    Mark,
    Chmod,
}

#[cfg(unix)]
fn unix_mode_fix(path: &str, remote: &BTreeMap<String, Entry>, resolve: &dyn Fn(&str) -> Option<PathBuf>) -> Result<Option<ModeFix>, String> {
    let Some(remote_exec) = remote.get(path).map(|entry| entry.executable) else { return Ok(None) };
    let Some(file) = resolve(path) else { return Ok(None) };
    if !file.is_file() {
        return Ok(None);
    }
    Ok(mode_fix(executable(&file)?, remote_exec))
}

fn executable(path: &Path) -> Result<Option<bool>, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Some(mode) = std::fs::metadata(path).ok().map(|meta| meta.permissions().mode()) else { return Ok(None) };
        // The owner runs the hook.
        let owner = if mode & 0o111 != 0 && mode & 0o100 == 0 {
            set_owner_execute(path)?;
            true
        } else {
            mode & 0o100 != 0
        };
        return Ok(Some(owner));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(None)
    }
}

fn write(target: &Path, bytes: &[u8], modified: Option<u64>, executable: Option<bool>) -> Result<(), String> {
    let previous = executable.is_none().then(|| std::fs::metadata(target).ok()).flatten().map(|meta| meta.permissions());
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(target).map_err(|e| format!("could not write {}: {e}", target.display()))?;
    (&file).write_all(bytes).map_err(|e| e.to_string())?;
    match executable {
        Some(bit) => apply_executable(&file, bit)?,
        None => {
            if let Some(perms) = previous {
                file.set_permissions(perms).map_err(|e| e.to_string())?;
            }
        }
    }
    if let Some(ms) = modified {
        let _ = file.set_modified(UNIX_EPOCH + Duration::from_millis(ms));
    }
    Ok(())
}

// A directory this process owns still allows chmod. Tests set this to stand in for EPERM.
#[cfg(all(unix, test))]
thread_local! {
    static DENY_OWNER_EXECUTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(unix)]
fn set_owner_execute(path: &Path) -> Result<(), String> {
    #[cfg(test)]
    if DENY_OWNER_EXECUTE.with(|deny| deny.get()) {
        return Err("Operation not permitted (os error 1)".into());
    }
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    perms.set_mode(perms.mode() | 0o100);
    std::fs::set_permissions(path, perms).map_err(|e| e.to_string())
}

fn apply_executable(file: &std::fs::File, executable: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file.metadata().map_err(|e| e.to_string())?.permissions();
        let mode = perms.mode();
        // The owner runs the hook. 0o111 would turn mode 600 into 711.
        perms.set_mode(if executable { mode | 0o100 } else { mode & !0o111 });
        file.set_permissions(perms).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        let _ = (file, executable);
    }
    Ok(())
}

pub fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn pack(bytes: &[u8]) -> Vec<u8> {
    let mut gz = GzEncoder::new(vec![], flate2::Compression::default());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

fn unpack(data: &str) -> Result<Vec<u8>, String> {
    let gz = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
    let mut bytes = vec![];
    GzDecoder::new(gz.as_slice()).read_to_end(&mut bytes).map_err(|e| format!("a synced file is corrupt: {e}"))?;
    Ok(bytes)
}

fn millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct MemHub(RefCell<BTreeMap<String, File>>);

    impl Hub for MemHub {
        fn list(&self, _: &str) -> Result<Vec<Entry>, String> {
            Ok(self.0.borrow().values().map(|f| f.entry.clone()).collect())
        }
        fn push(&self, _: &str, files: Vec<File>) -> Result<(), String> {
            for f in files {
                self.0.borrow_mut().insert(f.entry.path.clone(), f);
            }
            Ok(())
        }
        fn pull(&self, _: &str, paths: Vec<String>) -> Result<Vec<File>, String> {
            Ok(paths.iter().filter_map(|p| self.0.borrow().get(p).cloned()).collect())
        }
        fn mark_executable(&self, _: &str, repairs: Vec<ModeRepair>) -> Result<(), String> {
            let mut files = self.0.borrow_mut();
            for repair in repairs {
                if let Some(file) = files.get_mut(&repair.path) {
                    if file.entry.sha == repair.sha {
                        file.entry.executable = Some(true);
                    }
                }
            }
            Ok(())
        }
    }

    struct Side {
        home: PathBuf,
        checkout: Checkout,
        state: State,
    }

    impl Side {
        fn new(root: &Path, name: &str, folder: &str) -> Side {
            let home = root.join(name);
            let path = home.join(folder);
            std::fs::create_dir_all(&path).unwrap();
            let project = Project { key: "github.com/acme/app".into(), name: "app".into(), repo: "git@github.com:acme/app.git".into() };
            let checkout = Checkout { project, history: Some(home.join("history/app.jsonl")), claude_home: Some(home.join(".claude")), path };
            Side { checkout, home, state: State::default() }
        }
        fn claude(&self, rest: &str) -> PathBuf {
            claude_dir(self.checkout.claude_home.as_ref().unwrap(), &self.checkout.path).join(rest)
        }
        fn sync(&mut self, hub: &MemHub) -> Synced {
            sync(hub, &mut self.state, &self.checkout, SystemTime::now()).unwrap()
        }
    }

    fn put(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("pinned-sync-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn claude_names_a_project_folder_by_its_path() {
        assert_eq!(claude_dir(Path::new("/h/.claude"), Path::new("/Users/ana/my.app")), PathBuf::from("/h/.claude/projects/-Users-ana-my-app"));
    }

    #[test]
    fn a_project_without_a_claude_account_shares_no_claude_files_and_deletes_none() {
        let root = root("codex");
        let hub = MemHub::default();
        let (mut mac, mut mini) = (Side::new(&root, "mac", "app"), Side::new(&root, "mini", "app"));
        put(&mac.claude("memory/MEMORY.md"), "- deploys go through the VPS");
        put(&mac.checkout.path.join(".claude/skills/release/SKILL.md"), "---\nname: release\n---");
        mac.sync(&hub);
        mini.checkout.claude_home = None;
        assert_eq!(mini.sync(&hub).pulled, vec!["skills/release/SKILL.md"]);
        assert!(!root.join("mini/.claude").exists());
        mac.checkout.claude_home = None;
        assert_eq!(mac.sync(&hub), Synced::default(), "files left out here are not deleted elsewhere");
        assert!(hub.0.borrow()["claude/memory/MEMORY.md"].data.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_project_moved_to_another_account_takes_its_claude_files_there() {
        let root = root("account");
        let hub = MemHub::default();
        let mut mac = Side::new(&root, "mac", "app");
        put(&mac.claude("memory/MEMORY.md"), "- deploys go through the VPS");
        mac.sync(&hub);
        mac.checkout.claude_home = Some(root.join("mac/pinned/accounts/work"));
        assert_eq!(mac.sync(&hub).pulled, vec!["claude/memory/MEMORY.md"], "nothing is taken for deleted");
        assert_eq!(std::fs::read_to_string(mac.claude("memory/MEMORY.md")).unwrap(), "- deploys go through the VPS");
        assert!(hub.0.borrow()["claude/memory/MEMORY.md"].data.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_state_from_before_accounts_had_its_claude_files_in_the_default_home() {
        let root = root("legacy");
        let hub = MemHub::default();
        let mut mac = Side::new(&root, "mac", "app");
        put(&mac.claude("memory/MEMORY.md"), "- deploys go through the VPS");
        put(&mac.claude("s1.jsonl"), "{}");
        mac.sync(&hub);
        let file = root.join("sync.json");
        let legacy = |side: &Side| {
            let mut saved = serde_json::to_value(&side.state).unwrap();
            saved.as_object_mut().unwrap().remove("claude_homes");
            std::fs::write(&file, saved.to_string()).unwrap();
            State::load(&file, &side.home.join(".claude"))
        };
        std::fs::remove_file(mac.claude("s1.jsonl")).unwrap();
        mac.state = legacy(&mac);
        assert_eq!(mac.sync(&hub).pushed, vec!["claude/s1.jsonl"], "a session deleted here is deleted on the hub");
        assert!(hub.0.borrow()["claude/s1.jsonl"].data.is_none());
        mac.state = legacy(&mac);
        mac.checkout.claude_home = Some(root.join("mac/pinned/accounts/work"));
        assert_eq!(mac.sync(&hub).pulled, vec!["claude/memory/MEMORY.md"], "a project moved to another account before its first sync deletes nothing");
        assert!(hub.0.borrow()["claude/memory/MEMORY.md"].data.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn memory_sessions_and_skills_reach_a_machine_that_keeps_the_project_elsewhere() {
        let root = root("reach");
        let hub = MemHub::default();
        let (mut mac, mut mini) = (Side::new(&root, "mac", "projetos/app"), Side::new(&root, "mini", "pinned/app"));
        put(&mac.claude("memory/MEMORY.md"), "- deploys go through the VPS");
        put(&mac.claude("s1.jsonl"), "{\"type\":\"user\"}\n");
        put(&mac.claude("s1/subagents/a.jsonl"), "not shared");
        put(&mac.checkout.path.join(".claude/skills/release/SKILL.md"), "---\nname: release\n---");
        let pushed = mac.sync(&hub).pushed;
        assert_eq!(pushed, vec!["claude/memory/MEMORY.md", "claude/s1.jsonl", "skills/release/SKILL.md"]);
        assert_eq!(mini.sync(&hub).pulled, pushed);
        assert_eq!(std::fs::read_to_string(mini.claude("memory/MEMORY.md")).unwrap(), "- deploys go through the VPS");
        assert_eq!(std::fs::read_to_string(mini.checkout.path.join(".claude/skills/release/SKILL.md")).unwrap(), "---\nname: release\n---");
        assert_eq!((mac.sync(&hub), mini.sync(&hub)), (Synced::default(), Synced::default()), "nothing moves once both agree");

        put(&mini.claude("memory/MEMORY.md"), "- deploys go through CI");
        assert_eq!(mini.sync(&hub).pushed, vec!["claude/memory/MEMORY.md"]);
        assert_eq!(mac.sync(&hub).pulled, vec!["claude/memory/MEMORY.md"]);
        assert_eq!(std::fs::read_to_string(mac.claude("memory/MEMORY.md")).unwrap(), "- deploys go through CI");

        std::fs::remove_file(mac.claude("memory/MEMORY.md")).unwrap();
        assert_eq!(mac.sync(&hub).pushed, vec!["claude/memory/MEMORY.md"]);
        assert_eq!(mini.sync(&hub).pulled, vec!["claude/memory/MEMORY.md"]);
        assert!(!mini.claude("memory/MEMORY.md").exists(), "a deletion travels too");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn both_machines_keep_their_turns_in_the_history() {
        let root = root("history");
        let hub = MemHub::default();
        let (mut mac, mut mini) = (Side::new(&root, "mac", "app"), Side::new(&root, "mini", "app"));
        put(mac.checkout.history.as_ref().unwrap(), "{\"session\":\"a\",\"ended\":1}\n");
        mac.sync(&hub);
        put(mini.checkout.history.as_ref().unwrap(), "{\"session\":\"b\",\"ended\":2}\n");
        assert_eq!(mini.sync(&hub).pushed, vec![HISTORY]);
        mac.sync(&hub);
        let both = "{\"session\":\"a\",\"ended\":1}\n{\"session\":\"b\",\"ended\":2}\n";
        assert_eq!(std::fs::read_to_string(mini.checkout.history.as_ref().unwrap()).unwrap(), both);
        assert_eq!(std::fs::read_to_string(mac.checkout.history.as_ref().unwrap()).unwrap(), both);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_old_session_stays_home_and_a_moved_checkout_deletes_nothing() {
        let root = root("old");
        let hub = MemHub::default();
        let mut mac = Side::new(&root, "mac", "app");
        put(&mac.claude("old.jsonl"), "{}");
        std::fs::File::options().write(true).open(mac.claude("old.jsonl")).unwrap().set_modified(SystemTime::now() - Duration::from_secs(40 * 24 * 3600)).unwrap();
        put(&mac.claude("memory/notes.md"), "kept");
        assert_eq!(mac.sync(&hub).pushed, vec!["claude/memory/notes.md"]);

        mac.checkout.path = mac.home.join("moved/app");
        std::fs::create_dir_all(&mac.checkout.path).unwrap();
        assert_eq!(mac.sync(&hub).pulled, vec!["claude/memory/notes.md"], "the new folder takes the files, and nothing is deleted");
        assert!(hub.0.borrow()["claude/memory/notes.md"].data.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_file_named_outside_the_shared_folders_is_never_written() {
        let root = root("outside");
        let side = Side::new(&root, "mac", "app");
        let roots = Roots::new(&side.checkout);
        for path in ["claude/../../.ssh/authorized_keys", "claude/settings.json", "claude/s1/subagents/a.jsonl", "skills/../../x", "/etc/passwd", "other/x"] {
            assert_eq!(roots.local(path), None, "{path}");
        }
        assert_eq!(roots.local("claude/memory/a/b.md"), Some(side.claude("memory/a/b.md")));
        let _ = std::fs::remove_dir_all(root);
    }

    fn enable(home: &Path, id: &str, path: &Path, version: &str, sha: &str) {
        put(&home.join(".claude/settings.json"), &format!(r#"{{"enabledPlugins":{{"{id}":true,"off@demo":false,"market@place":true}}}}"#));
        let installed = serde_json::json!({ "plugins": {
            id: [
                { "installPath": home.join("plugins/old"), "version": "0.1.0" },
                { "installPath": path, "version": version, "gitCommitSha": sha },
            ],
            "off@demo": [{ "installPath": home.join("plugins/off"), "version": "1" }],
            "market@place": [{ "installPath": home.join("plugins/marketplaces/place/market"), "version": "9" }],
        }});
        put(&home.join(".claude/plugins/installed_plugins.json"), &installed.to_string());
    }

    #[test]
    fn a_plugin_enabled_on_one_machine_is_a_plugin_dir_on_the_other() {
        let root = root("plugins");
        let hub = MemHub::default();
        let (mac_home, mini_home) = (root.join("mac"), root.join("mini"));
        let install = mac_home.join("plugins/toy");
        put(&install.join(".claude-plugin/plugin.json"), r#"{"name":"toy"}"#);
        put(&install.join("skills/greet/SKILL.md"), "---\nname: greet\ndescription: Say hello\n---\n");
        put(&install.join("agents/helper.md"), "helps");
        put(&install.join("settings.json"), "token=1");
        put(&install.join(".env"), "SECRET=1");
        put(&install.join(".claude.json"), "{}");
        put(&install.join("secrets/key.pem"), "pem");
        put(&install.join("node_modules/pkg/index.js"), "nope");
        put(&mac_home.join("plugins/old/skills/old/SKILL.md"), "---\nname: old\n---\n");
        put(&mac_home.join("plugins/off/skills/hidden/SKILL.md"), "---\nname: hidden\n---\n");
        put(&mac_home.join("plugins/marketplaces/place/market/skills/src/SKILL.md"), "---\nname: src\n---\n");
        put(&mac_home.join(".claude/skills/ship/SKILL.md"), "---\nname: ship\ndescription: Ship the branch\n---\n");
        put(&mac_home.join(".claude/skills/synced/org/stale/SKILL.md"), "---\nname: stale\n---\n");
        enable(&mac_home, "toy@demo", &install, "1.2.3", "abc123");
        let (mac, mini) = (
            PluginShare { claude_home: mac_home.join(".claude"), dir: root.join("mac-synced") },
            PluginShare { claude_home: mini_home.join(".claude"), dir: root.join("mini-synced") },
        );
        let now = SystemTime::now();
        sync_plugins(&hub, &mut State::default(), &mac, now).unwrap();
        sync_plugins(&hub, &mut State::default(), &mini, now).unwrap();
        let mut paths: Vec<_> = hub.0.borrow().keys().cloned().collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "plugins/toy@demo/.claude-plugin/plugin.json",
                "plugins/toy@demo/.pinned-plugin.json",
                "plugins/toy@demo/agents/helper.md",
                "plugins/toy@demo/skills/greet/SKILL.md",
                "user-skills/ship/SKILL.md",
            ]
        );
        let skill = mini_home.join(".claude/skills/ship/SKILL.md");
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), "---\nname: ship\ndescription: Ship the branch\n---\n");
        let dirs = plugin_dirs(&mini.claude_home, &mini.dir);
        assert_eq!(dirs, vec![mini.dir.join("plugins/toy@demo")]);
        assert_eq!(plugin_dirs(&mac.claude_home, &mac.dir), Vec::<PathBuf>::new(), "the machine that already enables the plugin does not load the copy");
        let listed: Vec<_> = crate::commands::list(&mini.claude_home, &mini_home, &dirs)
            .into_iter()
            .filter(|c| c.source != "builtin")
            .map(|c| (c.name, c.description, c.source, c.skill))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("ship".into(), "Ship the branch".into(), "user", true),
                ("toy:greet".into(), "Say hello".into(), "plugin", true),
            ]
        );
        let marker: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(mini.dir.join("plugins/toy@demo/.pinned-plugin.json")).unwrap()).unwrap();
        assert_eq!(marker, serde_json::json!({"version": "1.2.3", "gitCommitSha": "abc123"}));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn disabling_a_plugin_removes_it_from_the_other_machine() {
        let root = root("disable");
        let hub = MemHub::default();
        let (mac_home, mini_home) = (root.join("mac"), root.join("mini"));
        let install = mac_home.join("plugins/toy");
        put(&install.join(".claude-plugin/plugin.json"), r#"{"name":"toy"}"#);
        put(&install.join("skills/greet/SKILL.md"), "---\nname: greet\ndescription: Say hello\n---\n");
        enable(&mac_home, "toy@demo", &install, "1.2.3", "abc123");
        let (mac, mini) = (
            PluginShare { claude_home: mac_home.join(".claude"), dir: root.join("mac-synced") },
            PluginShare { claude_home: mini_home.join(".claude"), dir: root.join("mini-synced") },
        );
        let now = SystemTime::now();
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert!(mini.dir.join("plugins/toy@demo/skills/greet/SKILL.md").is_file());
        put(&mac_home.join(".claude/settings.json"), r#"{"enabledPlugins":{"toy@demo":false}}"#);
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert!(!mini.dir.join("plugins/toy@demo/skills/greet/SKILL.md").exists());
        assert_eq!(plugin_dirs(&mini.claude_home, &mini.dir), Vec::<PathBuf>::new());
        assert!(hub.0.borrow()["plugins/toy@demo/skills/greet/SKILL.md"].data.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_old_hub_that_rejects_plugins_leaves_the_local_copy_to_launch() {
        let root = root("oldhub");
        let home = root.join("mac/.claude");
        let dir = root.join("synced");
        let plugin = dir.join("plugins/toy@demo");
        put(&plugin.join(".claude-plugin/plugin.json"), r#"{"name":"toy"}"#);
        put(&plugin.join("skills/greet/SKILL.md"), "---\nname: greet\ndescription: Say hello\n---\n");
        let share = PluginShare { claude_home: home, dir: dir.clone() };
        let err = sync_plugins(&OldHub, &mut State::default(), &share, SystemTime::now()).unwrap_err();
        assert_eq!((old_hub(&err), plugin_dirs(&share.claude_home, &share.dir)), (true, vec![plugin]));
        let _ = std::fs::remove_dir_all(root);
    }

    fn texts(hub: &MemHub) -> BTreeMap<String, String> {
        hub.0
            .borrow()
            .iter()
            .map(|(path, file)| {
                let text = file.data.as_ref().and_then(|data| unpack(data).ok()).and_then(|bytes| String::from_utf8(bytes).ok()).unwrap_or_default();
                (path.clone(), text)
            })
            .collect()
    }

    fn stamp(path: &Path, when: SystemTime) {
        std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
    }

    #[test]
    fn secret_files_and_outside_symlinks_stay_on_the_machine() {
        let root = root("secrets");
        let hub = MemHub::default();
        let mac_home = root.join("mac");
        let outside = root.join("outside");
        put(&outside.join("id_rsa"), "PRIVATE-KEY");
        put(&outside.join("SKILL.md"), "STOLEN-SKILL");
        put(&outside.join("notes.md"), "STOLEN-NOTES");
        let install = mac_home.join("plugins/toy");
        put(&install.join(".claude-plugin/plugin.json"), r#"{"name":"toy"}"#);
        put(&install.join("skills/greet/SKILL.md"), "---\nname: greet\ndescription: Say hello\n---\n");
        for (rel, body) in [
            ("Settings.json", "CASE-SETTINGS"),
            (".ENV", "CASE-ENV"),
            (".env.local", "ENV-LOCAL"),
            ("Credentials.JSON", "CASE-CREDENTIALS"),
            (".Claude.json", "CASE-CLAUDE"),
            ("Installed_Plugins.json", "CASE-INSTALLED"),
            ("settings.local.json", "LOCAL-SETTINGS"),
            ("cert.PEM", "CASE-PEM"),
            ("id.KEY", "CASE-KEY"),
            ("Secrets/token.txt", "DIR-SECRETS"),
            (".SECRETS/note.txt", "DIR-DOT-SECRETS"),
            ("target/release/toy", "TREE-TARGET"),
            ("dist/app.js", "TREE-DIST"),
            ("build/out.js", "TREE-BUILD"),
            ("vendor/lib.js", "TREE-VENDOR"),
        ] {
            put(&install.join(rel), body);
        }
        std::fs::create_dir_all(install.join("agents")).unwrap();
        let linked_real = root.join("linked-real");
        put(&linked_real.join(".claude-plugin/plugin.json"), r#"{"name":"linked"}"#);
        put(&linked_real.join("readme.txt"), "LINKED-SECRET");
        put(&mac_home.join(".claude/skills/ship/SKILL.md"), "---\nname: ship\ndescription: Ship the branch\n---\n");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, install.join("skills/escape")).unwrap();
            std::os::unix::fs::symlink(outside.join("id_rsa"), install.join("agents/helper.md")).unwrap();
            std::os::unix::fs::symlink(&linked_real, mac_home.join("plugins/linked")).unwrap();
            std::os::unix::fs::symlink(&outside, mac_home.join(".claude/skills/stolen")).unwrap();
        }
        let installed = serde_json::json!({ "plugins": {
            "toy@demo": [{ "installPath": install, "version": "1.2.3", "gitCommitSha": "abc123" }],
            "linked@demo": [{ "installPath": mac_home.join("plugins/linked"), "version": "1", "gitCommitSha": "def" }],
        }});
        put(&mac_home.join(".claude/settings.json"), r#"{"enabledPlugins":{"toy@demo":true,"linked@demo":true}}"#);
        put(&mac_home.join(".claude/plugins/installed_plugins.json"), &installed.to_string());
        let ssh = root.join("ssh");
        put(&ssh.join("id_rsa"), "ROOT-PRIVATE");
        put(&ssh.join("config"), "ROOT-CONFIG");
        let rooted = root.join("rooted");
        std::fs::create_dir_all(rooted.join(".claude")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&ssh, rooted.join(".claude/skills")).unwrap();
        let mac = PluginShare { claude_home: mac_home.join(".claude"), dir: root.join("mac-synced") };
        let rooted_share = PluginShare { claude_home: rooted.join(".claude"), dir: root.join("rooted-synced") };
        let now = SystemTime::now();
        sync_plugins(&hub, &mut State::default(), &mac, now).unwrap();
        sync_plugins(&hub, &mut State::default(), &rooted_share, now).unwrap();
        let stored = texts(&hub);
        let markers = [
            "CASE-SETTINGS",
            "CASE-ENV",
            "ENV-LOCAL",
            "CASE-CREDENTIALS",
            "CASE-CLAUDE",
            "CASE-INSTALLED",
            "LOCAL-SETTINGS",
            "CASE-PEM",
            "CASE-KEY",
            "DIR-SECRETS",
            "DIR-DOT-SECRETS",
            "TREE-TARGET",
            "TREE-DIST",
            "TREE-BUILD",
            "TREE-VENDOR",
            "PRIVATE-KEY",
            "STOLEN-SKILL",
            "STOLEN-NOTES",
            "LINKED-SECRET",
            "ROOT-PRIVATE",
            "ROOT-CONFIG",
        ];
        let leaked: Vec<String> = stored
            .iter()
            .flat_map(|(path, text)| markers.iter().filter(move |marker| text.contains(**marker)).map(move |marker| format!("{path} contains {marker}")))
            .collect();
        assert_eq!(leaked, Vec::<String>::new(), "a secret left the machine");
        assert_eq!(stored["plugins/toy@demo/skills/greet/SKILL.md"], "---\nname: greet\ndescription: Say hello\n---\n");
        assert_eq!(stored["user-skills/ship/SKILL.md"], "---\nname: ship\ndescription: Ship the branch\n---\n");
        assert_eq!(std::fs::read_to_string(ssh.join("id_rsa")).unwrap(), "ROOT-PRIVATE");
        assert!(!ssh.join("ship").exists(), "a pulled skill was written through the skills symlink");
        assert_eq!(std::fs::read_to_string(outside.join("id_rsa")).unwrap(), "PRIVATE-KEY");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_hub_refuses_secret_names_and_paths_that_leave_the_tree() {
        let refused = [
            "plugins/toy@demo/Settings.json",
            "plugins/toy@demo/.ENV",
            "plugins/toy@demo/.env.local",
            "plugins/toy@demo/Credentials.JSON",
            "plugins/toy@demo/.Claude.json",
            "plugins/toy@demo/Installed_Plugins.json",
            "plugins/toy@demo/settings.local.json",
            "plugins/toy@demo/cert.PEM",
            "plugins/toy@demo/id.KEY",
            "plugins/toy@demo/Secrets/token.txt",
            "plugins/toy@demo/.SECRETS/note.txt",
            "plugins/toy@demo/../../.ssh/id_rsa",
            "plugins/../.ssh/id_rsa",
            "/etc/passwd",
            "user-skills/../../.ssh/id_rsa",
            "user-skills/stolen/../../../.ssh/id_rsa",
            "plugins/toy@demo/target/release/toy",
            "plugins/toy@demo/dist/app.js",
            "plugins/toy@demo/build/out.js",
            "plugins/toy@demo/vendor/lib.js",
            "plugins/toy@demo/node_modules/pkg/index.js",
            "user-skills/synced/org/SKILL.md",
            "claude/settings.json",
        ];
        let accepted: Vec<_> = refused.into_iter().filter(|path| accepts(PLUGINS_KEY, path)).collect();
        assert_eq!(accepted, Vec::<&str>::new());
        assert!(accepts(PLUGINS_KEY, "plugins/toy@demo/skills/greet/SKILL.md"));
        assert!(accepts(PLUGINS_KEY, "user-skills/ship/SKILL.md"));
        assert!(!accepts("github.com/acme/app", "plugins/toy@demo/skills/greet/SKILL.md"));
    }

    #[test]
    fn a_skill_this_machine_wrote_is_not_replaced_or_removed() {
        let root = root("local-skill");
        let hub = MemHub::default();
        let (mac_home, mini_home) = (root.join("mac"), root.join("mini"));
        let (mac, mini) = (
            PluginShare { claude_home: mac_home.join(".claude"), dir: root.join("mac-synced") },
            PluginShare { claude_home: mini_home.join(".claude"), dir: root.join("mini-synced") },
        );
        let older = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let newer = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);
        put(&mini_home.join(".claude/skills/notes/SKILL.md"), "from mini");
        stamp(&mini_home.join(".claude/skills/notes/SKILL.md"), older);
        put(&mac_home.join(".claude/skills/notes/SKILL.md"), "from mac");
        stamp(&mac_home.join(".claude/skills/notes/SKILL.md"), newer);
        put(&mac_home.join(".claude/skills/ship/SKILL.md"), "ship v1");
        stamp(&mac_home.join(".claude/skills/ship/SKILL.md"), older);
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!(std::fs::read_to_string(mini_home.join(".claude/skills/notes/SKILL.md")).unwrap(), "from mini");
        assert_eq!(std::fs::read_to_string(mini_home.join(".claude/skills/ship/SKILL.md")).unwrap(), "ship v1");
        put(&mac_home.join(".claude/skills/ship/SKILL.md"), "ship v2");
        stamp(&mac_home.join(".claude/skills/ship/SKILL.md"), newer);
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!(std::fs::read_to_string(mini_home.join(".claude/skills/ship/SKILL.md")).unwrap(), "ship v2");
        std::fs::remove_file(mac_home.join(".claude/skills/notes/SKILL.md")).unwrap();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!(std::fs::read_to_string(mini_home.join(".claude/skills/notes/SKILL.md")).unwrap(), "from mini");
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        assert!(!mac_home.join(".claude/skills/notes/SKILL.md").exists(), "a skill mac deleted came back from mini");
        std::fs::remove_file(mac_home.join(".claude/skills/ship/SKILL.md")).unwrap();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert!(!mini_home.join(".claude/skills/ship/SKILL.md").exists(), "a skill mini did not write is removed when mac deletes it");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn disabling_a_plugin_does_not_delete_outside_the_synced_copy() {
        let root = root("keep");
        let hub = MemHub::default();
        let (mac_home, mini_home) = (root.join("mac"), root.join("mini"));
        let install = mac_home.join("plugins/toy");
        put(&install.join(".claude-plugin/plugin.json"), r#"{"name":"toy"}"#);
        put(&install.join("skills/greet/SKILL.md"), "---\nname: greet\ndescription: Say hello\n---\n");
        enable(&mac_home, "toy@demo", &install, "1.2.3", "abc123");
        put(&mini_home.join("plugins/toy/skills/greet/SKILL.md"), "hand installed");
        put(&mini_home.join("keep.txt"), "keep");
        put(&mini_home.join(".claude/skills/hand/SKILL.md"), "hand skill");
        let (mac, mini) = (
            PluginShare { claude_home: mac_home.join(".claude"), dir: root.join("mac-synced") },
            PluginShare { claude_home: mini_home.join(".claude"), dir: root.join("mini-synced") },
        );
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        let outside = root.join("precious");
        put(&outside.join("precious.txt"), "precious");
        std::fs::remove_dir_all(mac.dir.join("plugins/toy@demo")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, mac.dir.join("plugins/toy@demo")).unwrap();
        put(&mac_home.join(".claude/settings.json"), r#"{"enabledPlugins":{"toy@demo":false}}"#);
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!(std::fs::read_to_string(outside.join("precious.txt")).unwrap(), "precious");
        assert_eq!(std::fs::read_to_string(mini_home.join("plugins/toy/skills/greet/SKILL.md")).unwrap(), "hand installed");
        assert_eq!(std::fs::read_to_string(mini_home.join("keep.txt")).unwrap(), "keep");
        assert_eq!(std::fs::read_to_string(mini_home.join(".claude/skills/hand/SKILL.md")).unwrap(), "hand skill");
        assert!(!mini.dir.join("plugins/toy@demo/skills/greet/SKILL.md").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_enabled_copies_with_the_same_mtime_do_not_trade_edits() {
        let root = root("pong");
        let hub = MemHub::default();
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let (mac_home, mini_home) = (root.join("mac"), root.join("mini"));
        for (home, body) in [(&mac_home, "mac-body-aaaa"), (&mini_home, "mini-body-bbbb-longer")] {
            let install = home.join("plugins/toy");
            put(&install.join(".claude-plugin/plugin.json"), r#"{"name":"toy"}"#);
            put(&install.join("skills/greet/SKILL.md"), body);
            stamp(&install.join(".claude-plugin/plugin.json"), when);
            stamp(&install.join("skills/greet/SKILL.md"), when);
            enable(home, "toy@demo", &install, "1.2.3", "abc123");
        }
        let (mac, mini) = (
            PluginShare { claude_home: mac_home.join(".claude"), dir: root.join("mac-synced") },
            PluginShare { claude_home: mini_home.join(".claude"), dir: root.join("mini-synced") },
        );
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        let again = sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        assert_eq!(again.pushed, Vec::<String>::new(), "the machines kept publishing the same file");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_file_past_the_limit_is_not_pushed() {
        let root = root("limit");
        let hub = MemHub::default();
        let home = root.join("mac");
        let install = home.join("plugins/toy");
        put(&install.join(".claude-plugin/plugin.json"), r#"{"name":"toy"}"#);
        put(&install.join("skills/greet/SKILL.md"), "small");
        let mut blob = vec![0u8; FILE_LIMIT as usize + (1 << 20)];
        let mut seed = 0x1234_5678_9abc_def0u64;
        for byte in &mut blob {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *byte = seed as u8;
        }
        std::fs::write(install.join("skills/greet/blob.bin"), &blob).unwrap();
        enable(&home, "toy@demo", &install, "1.2.3", "abc123");
        let share = PluginShare { claude_home: home.join(".claude"), dir: root.join("synced") };
        let synced = sync_plugins(&hub, &mut State::default(), &share, SystemTime::now()).unwrap();
        assert!(synced.pushed.iter().any(|path| path.ends_with("SKILL.md")));
        assert!(synced.pushed.iter().all(|path| !path.contains("blob.bin")), "a file over the limit left the machine");
        assert!(hub.0.borrow().keys().all(|path| !path.contains("blob.bin")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    const HOOK: &str = "plugins/pstack@local/hooks/session-start.sh";

    #[cfg(unix)]
    #[derive(Default)]
    struct RacingHub {
        files: RefCell<BTreeMap<String, File>>,
        /// Another machine stores this after list and before the repair is applied.
        ahead: RefCell<Option<File>>,
    }

    #[cfg(unix)]
    impl Hub for RacingHub {
        fn list(&self, _: &str) -> Result<Vec<Entry>, String> {
            Ok(self.files.borrow().values().map(|file| file.entry.clone()).collect())
        }
        fn push(&self, _: &str, files: Vec<File>) -> Result<(), String> {
            for file in files {
                if file.entry.path == HOOK {
                    if let Some(newer) = self.ahead.borrow_mut().take() {
                        self.files.borrow_mut().insert(HOOK.into(), newer);
                    }
                }
                self.files.borrow_mut().insert(file.entry.path.clone(), file);
            }
            Ok(())
        }
        fn pull(&self, _: &str, paths: Vec<String>) -> Result<Vec<File>, String> {
            Ok(paths.iter().filter_map(|path| self.files.borrow().get(path).cloned()).collect())
        }
        fn mark_executable(&self, _: &str, repairs: Vec<ModeRepair>) -> Result<(), String> {
            if repairs.iter().any(|repair| repair.path == HOOK) {
                if let Some(newer) = self.ahead.borrow_mut().take() {
                    self.files.borrow_mut().insert(HOOK.into(), newer);
                }
            }
            let mut files = self.files.borrow_mut();
            for repair in repairs {
                if let Some(file) = files.get_mut(&repair.path) {
                    if file.entry.sha == repair.sha {
                        file.entry.executable = Some(true);
                    }
                }
            }
            Ok(())
        }
    }

    #[cfg(unix)]
    fn stored_hook(bytes: &[u8], modified: u64, executable: Option<bool>) -> File {
        use base64::Engine;
        let data = pack(bytes);
        File {
            entry: Entry { path: HOOK.into(), sha: sha(bytes), modified, size: data.len() as u64, executable },
            data: Some(base64::engine::general_purpose::STANDARD.encode(data)),
        }
    }

    #[cfg(unix)]
    fn mode_bits(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    fn executable_hook(root: &Path) -> (PathBuf, PluginShare, PluginShare) {
        let (mac_home, mini_home) = (root.join("mac"), root.join("mini"));
        let install = mac_home.join("plugins/pstack");
        put(&install.join(".claude-plugin/plugin.json"), r#"{"name":"pstack"}"#);
        let hook = install.join("hooks/session-start.sh");
        put(&hook, "#!/bin/sh\necho session-start\n");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        enable(&mac_home, "pstack@local", &install, "0.9.82", "abc123");
        let mac = PluginShare { claude_home: mac_home.join(".claude"), dir: root.join("mac-synced") };
        let mini = PluginShare { claude_home: mini_home.join(".claude"), dir: root.join("mini-synced") };
        (hook, mac, mini)
    }

    #[cfg(unix)]
    #[test]
    fn an_executable_hook_stays_executable_on_the_other_machine() {
        let root = root("hook-mode");
        let hub = MemHub::default();
        let (_, mac, mini) = executable_hook(&root);
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        let source = mac.dir.join("plugins/pstack@local/hooks/session-start.sh");
        let dest = mini.dir.join("plugins/pstack@local/hooks/session-start.sh");
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho session-start\n");
        assert_eq!(mode_bits(&source) & 0o111, 0o111, "source mirror mode is {:o}", mode_bits(&source));
        assert_eq!(mode_bits(&dest) & 0o111, 0o100, "receiver hook mode is {:o}", mode_bits(&dest));
        let plain = mini.dir.join("plugins/pstack@local/.claude-plugin/plugin.json");
        assert_eq!(mode_bits(&plain) & 0o111, 0, "plain file mode is {:o}", mode_bits(&plain));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_hook_already_synced_as_plain_text_becomes_executable_when_the_bytes_match() {
        let root = root("hook-heal");
        let hub = MemHub::default();
        let (_, mac, mini) = executable_hook(&root);
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        let source = mac.dir.join("plugins/pstack@local/hooks/session-start.sh");
        let dest = mini.dir.join("plugins/pstack@local/hooks/session-start.sh");
        put(&dest, "#!/bin/sh\necho session-start\n");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
        std::fs::File::options().write(true).open(&dest).unwrap().set_modified(modified).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), std::fs::read(&source).unwrap());
        assert_eq!(mode_bits(&dest), 0o644);
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!(mode_bits(&dest) & 0o111, 0o100, "resync left the hook at {:o}", mode_bits(&dest));
        let again = sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!(again.pushed, Vec::<String>::new());
        assert_eq!(again.pulled, Vec::<String>::new());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    fn publish_hook(home: &Path, mode: u32, when: SystemTime) {
        let install = home.join("plugins/pstack");
        put(&install.join(".claude-plugin/plugin.json"), r#"{"name":"pstack"}"#);
        let hook = install.join("hooks/session-start.sh");
        put(&hook, "#!/bin/sh\necho session-start\n");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(mode)).unwrap();
        stamp(&hook, when);
        stamp(&install.join(".claude-plugin/plugin.json"), when);
        enable(home, "pstack@local", &install, "0.9.82", "abc123");
    }

    fn omit_executable(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Array(items) => {
                for item in items {
                    omit_executable(item);
                }
            }
            serde_json::Value::Object(map) => {
                map.remove("executable");
                for item in map.values_mut() {
                    omit_executable(item);
                }
            }
            _ => {}
        }
    }

    fn round_trip_without_executable<T: serde::Serialize + serde::de::DeserializeOwned>(value: T) -> T {
        let mut json = serde_json::to_value(&value).unwrap();
        omit_executable(&mut json);
        serde_json::from_value(json).unwrap()
    }

    #[derive(Default)]
    struct OmittingHub(RefCell<BTreeMap<String, File>>);

    impl Hub for OmittingHub {
        fn list(&self, _: &str) -> Result<Vec<Entry>, String> {
            let entries: Vec<Entry> = self.0.borrow().values().map(|file| file.entry.clone()).collect();
            Ok(round_trip_without_executable(entries))
        }
        fn push(&self, _: &str, files: Vec<File>) -> Result<(), String> {
            for file in files {
                self.0.borrow_mut().insert(file.entry.path.clone(), file);
            }
            Ok(())
        }
        fn pull(&self, _: &str, paths: Vec<String>) -> Result<Vec<File>, String> {
            let files: Vec<File> = paths.iter().filter_map(|path| self.0.borrow().get(path).cloned()).collect();
            Ok(round_trip_without_executable(files))
        }
        fn mark_executable(&self, _: &str, _: Vec<ModeRepair>) -> Result<(), String> {
            Err("this hub does not share executable bits".into())
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_newer_plain_copy_with_the_same_bytes_does_not_clear_the_executable_hook() {
        let root = root("hook-sticky");
        let hub = MemHub::default();
        let older = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let newer = older + Duration::from_secs(120);
        let (mac_home, mini_home) = (root.join("mac"), root.join("mini"));
        publish_hook(&mac_home, 0o755, older);
        publish_hook(&mini_home, 0o644, newer);
        let (mac, mini) = (
            PluginShare { claude_home: mac_home.join(".claude"), dir: root.join("mac-synced") },
            PluginShare { claude_home: mini_home.join(".claude"), dir: root.join("mini-synced") },
        );
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        let mac_hook = mac.dir.join("plugins/pstack@local/hooks/session-start.sh");
        let mini_hook = mini.dir.join("plugins/pstack@local/hooks/session-start.sh");
        assert_eq!(std::fs::read(&mac_hook).unwrap(), b"#!/bin/sh\necho session-start\n");
        assert_eq!(std::fs::read(&mini_hook).unwrap(), b"#!/bin/sh\necho session-start\n");
        assert_eq!(mode_bits(&mac_hook) & 0o100, 0o100, "older executable copy is {:o}", mode_bits(&mac_hook));
        assert_eq!(mode_bits(&mini_hook) & 0o100, 0o100, "newer plain copy is {:o}", mode_bits(&mini_hook));
        let stored = serde_json::to_value(&hub.0.borrow()["plugins/pstack@local/hooks/session-start.sh"].entry).unwrap();
        assert_eq!(stored["executable"], serde_json::json!(true));
        let mac_again = sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        let mini_again = sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!((mac_again.pushed.as_slice(), mac_again.pulled.as_slice()), (&[][..], &[][..]));
        assert_eq!((mini_again.pushed.as_slice(), mini_again.pulled.as_slice()), (&[][..], &[][..]));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn chmod_on_the_install_reaches_the_receiver_without_a_new_mtime() {
        let root = root("hook-chmod");
        let hub = MemHub::default();
        let (hook, mac, mini) = executable_hook(&root);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        let source = mac.dir.join("plugins/pstack@local/hooks/session-start.sh");
        let dest = mini.dir.join("plugins/pstack@local/hooks/session-start.sh");
        assert_eq!(mode_bits(&dest) & 0o111, 0, "receiver started at {:o}", mode_bits(&dest));
        let mtime = std::fs::metadata(&hook).unwrap().modified().unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::File::options().write(true).open(&hook).unwrap().set_modified(mtime).unwrap();
        assert_eq!(std::fs::metadata(&hook).unwrap().modified().unwrap(), mtime);
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        assert_eq!(mode_bits(&source) & 0o100, 0o100, "mirror after chmod is {:o}", mode_bits(&source));
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho session-start\n");
        assert_eq!(mode_bits(&dest) & 0o100, 0o100, "receiver after chmod is {:o}", mode_bits(&dest));
        let mac_again = sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        let mini_again = sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!((mac_again.pushed.as_slice(), mac_again.pulled.as_slice()), (&[][..], &[][..]));
        assert_eq!((mini_again.pushed.as_slice(), mini_again.pulled.as_slice()), (&[][..], &[][..]));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_changed_hook_can_drop_the_executable_bit() {
        let root = root("hook-clear");
        let hub = MemHub::default();
        let (hook, mac, mini) = executable_hook(&root);
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        let dest = mini.dir.join("plugins/pstack@local/hooks/session-start.sh");
        assert_eq!(mode_bits(&dest) & 0o100, 0o100, "receiver hook mode is {:o}", mode_bits(&dest));
        std::fs::write(&hook, "#!/bin/sh\necho changed\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o644)).unwrap();
        stamp(&hook, SystemTime::UNIX_EPOCH + Duration::from_secs(1_900_000_000));
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho changed\n");
        assert_eq!(mode_bits(&dest) & 0o111, 0, "changed hook mode is {:o}", mode_bits(&dest));
        let stored = serde_json::to_value(&hub.0.borrow()["plugins/pstack@local/hooks/session-start.sh"].entry).unwrap();
        assert_eq!(stored["executable"], serde_json::json!(false));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn an_old_hub_that_omits_the_executable_field_is_not_pushed_every_sync() {
        let omit_root = root("old-hub-mode");
        let hub = OmittingHub::default();
        let (_, mac, _) = executable_hook(&omit_root);
        let mut state = State::default();
        let now = SystemTime::now();
        let first = sync_plugins(&hub, &mut state, &mac, now).unwrap();
        assert!(first.pushed.iter().any(|path| path.ends_with("session-start.sh")));
        let second = sync_plugins(&hub, &mut state, &mac, now).unwrap();
        assert_eq!((second.pushed.as_slice(), second.pulled.as_slice()), (&[][..], &[][..]));
        let third = sync_plugins(&hub, &mut state, &mac, now).unwrap();
        assert_eq!((third.pushed.as_slice(), third.pulled.as_slice()), (&[][..], &[][..]));
        let _ = std::fs::remove_dir_all(omit_root);

        let repair_root = root("mode-repair-once");
        let hub = MemHub::default();
        let published = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let local = SystemTime::UNIX_EPOCH + Duration::from_secs(50);
        let (mini_home, mac_home) = (repair_root.join("mini"), repair_root.join("mac"));
        publish_hook(&mini_home, 0o644, published);
        publish_hook(&mac_home, 0o755, local);
        let mini = PluginShare { claude_home: mini_home.join(".claude"), dir: repair_root.join("mini-synced") };
        let mac = PluginShare { claude_home: mac_home.join(".claude"), dir: repair_root.join("mac-synced") };
        sync_plugins(&hub, &mut State::default(), &mini, now).unwrap();
        let before = hub.0.borrow()[HOOK].clone();
        assert_eq!(before.entry.executable, Some(false));
        let repaired = sync_plugins(&hub, &mut State::default(), &mac, now).unwrap();
        let stored = hub.0.borrow()[HOOK].clone();
        assert!(!repaired.pushed.iter().any(|path| path == HOOK), "a mode repair sends no file bytes");
        assert_eq!(stored.entry.modified, before.entry.modified);
        assert_eq!(stored.entry.sha, before.entry.sha);
        assert_eq!(stored.data, before.data);
        assert_eq!(stored.entry.executable, Some(true));
        let again = sync_plugins(&hub, &mut State::default(), &mac, now).unwrap();
        let after = hub.0.borrow()[HOOK].clone();
        assert_eq!((again.pushed.as_slice(), again.pulled.as_slice()), (&[][..], &[][..]));
        assert_eq!(after, stored);
        let _ = std::fs::remove_dir_all(repair_root);
    }

    #[cfg(unix)]
    #[test]
    fn a_mode_repair_does_not_replace_bytes_a_later_edit_already_published() {
        let root = root("mode-repair-race");
        let hub = RacingHub::default();
        let published = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let local = SystemTime::UNIX_EPOCH + Duration::from_secs(50);
        let (mini_home, mac_home) = (root.join("mini"), root.join("mac"));
        publish_hook(&mini_home, 0o644, published);
        publish_hook(&mac_home, 0o755, local);
        let mini = PluginShare { claude_home: mini_home.join(".claude"), dir: root.join("mini-synced") };
        let mac = PluginShare { claude_home: mac_home.join(".claude"), dir: root.join("mac-synced") };
        let now = SystemTime::now();
        sync_plugins(&hub, &mut State::default(), &mini, now).unwrap();
        let newer = b"#!/bin/sh\necho edited\n";
        *hub.ahead.borrow_mut() = Some(stored_hook(newer, 200, Some(false)));
        sync_plugins(&hub, &mut State::default(), &mac, now).unwrap();
        let stored = hub.files.borrow()[HOOK].clone();
        assert_eq!(unpack(stored.data.as_deref().unwrap()).unwrap(), newer);
        assert_eq!(stored.entry.sha, sha(newer));
        assert_eq!(stored.entry.modified, 200);
        assert_eq!(stored.entry.executable, Some(false));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_mode_only_heal_chmods_the_existing_hook() {
        let root = root("hook-chmod-only");
        let hub = MemHub::default();
        let (_, mac, mini) = executable_hook(&root);
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        let dest = mini.dir.join(HOOK);
        put(&dest, "#!/bin/sh\necho session-start\n");
        let stamped = SystemTime::UNIX_EPOCH + Duration::from_secs(80);
        stamp(&dest, stamped);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o444)).unwrap();
        let healed = sync_plugins(&hub, &mut mini_state, &mini, now).expect("mode heal chmods a file that is not writable");
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho session-start\n");
        assert_eq!(mode_bits(&dest), 0o544, "healed hook mode is {:o}", mode_bits(&dest));
        assert_eq!(std::fs::metadata(&dest).unwrap().modified().unwrap(), stamped);
        assert!(!healed.pulled.iter().any(|path| path == HOOK), "a mode heal does not fetch the hook");
        assert!(!healed.pushed.iter().any(|path| path == HOOK), "a mode heal does not send the hook");
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_receiver_marks_the_hook_owner_executable_only() {
        let root = root("hook-owner");
        let hub = MemHub::default();
        let (_, mac, mini) = executable_hook(&root);
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        let dest = mini.dir.join("plugins/pstack@local/hooks/session-start.sh");
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho session-start\n");
        assert_eq!(mode_bits(&dest) & 0o111, 0o100, "receiver hook mode is {:o}", mode_bits(&dest));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_synced_project_file_does_not_gain_an_executable_bit() {
        let root = root("project-mode");
        let hub = MemHub::default();
        let (mut mac, mut mini) = (Side::new(&root, "mac", "app"), Side::new(&root, "mini", "app"));
        let memory = mac.claude("memory/MEMORY.md");
        put(&memory, "- notes");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&memory, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(mode_bits(&memory) & 0o111, 0o111, "source mode is {:o}", mode_bits(&memory));
        mac.sync(&hub);
        let path = "claude/memory/MEMORY.md";
        assert_eq!(hub.0.borrow()[path].entry.executable, None, "a project push carries no mode");
        mini.sync(&hub);
        let dest = mini.claude("memory/MEMORY.md");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "- notes");
        assert_eq!(mode_bits(&dest) & 0o111, 0, "project file mode is {:o}", mode_bits(&dest));
        let next = b"- notes\nfrom the hub\n";
        {
            use base64::Engine;
            let mut files = hub.0.borrow_mut();
            let file = files.get_mut(path).unwrap();
            let packed = pack(next);
            file.data = Some(base64::engine::general_purpose::STANDARD.encode(&packed));
            file.entry.sha = sha(next);
            file.entry.size = packed.len() as u64;
            file.entry.modified += 1;
            file.entry.executable = Some(true);
        }
        mini.sync(&hub);
        assert_eq!(std::fs::read(&dest).unwrap(), next);
        assert_eq!(mode_bits(&dest) & 0o111, 0, "a project pull ignores a stored executable bit, mode is {:o}", mode_bits(&dest));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    const USER_SKILL: &str = "user-skills/greet/SKILL.md";

    #[cfg(unix)]
    struct MarkHub {
        inner: MemHub,
        fail: std::cell::Cell<bool>,
    }

    #[cfg(unix)]
    impl Hub for MarkHub {
        fn list(&self, project: &str) -> Result<Vec<Entry>, String> {
            self.inner.list(project)
        }
        fn push(&self, project: &str, files: Vec<File>) -> Result<(), String> {
            self.inner.push(project, files)
        }
        fn pull(&self, project: &str, paths: Vec<String>) -> Result<Vec<File>, String> {
            self.inner.pull(project, paths)
        }
        fn mark_executable(&self, project: &str, repairs: Vec<ModeRepair>) -> Result<(), String> {
            if self.fail.get() {
                return Err("the hub did not answer within 60 seconds".into());
            }
            self.inner.mark_executable(project, repairs)
        }
    }

    #[cfg(unix)]
    struct DenyOwnerExecute;

    #[cfg(unix)]
    impl DenyOwnerExecute {
        fn arm() -> Self {
            super::DENY_OWNER_EXECUTE.with(|deny| deny.set(true));
            Self
        }
    }

    #[cfg(unix)]
    impl Drop for DenyOwnerExecute {
        fn drop(&mut self) {
            super::DENY_OWNER_EXECUTE.with(|deny| deny.set(false));
        }
    }

    #[cfg(unix)]
    fn saved_agreed(state: &State, share: &PluginShare, file: &Path) -> BTreeMap<String, String> {
        state.save(file);
        let loaded = State::load(file, &file.with_extension("legacy"));
        let key = format!("{PLUGINS_KEY} {}", share.claude_home.display());
        loaded.agreed.get(&key).cloned().unwrap_or_default()
    }

    #[cfg(unix)]
    #[test]
    fn a_group_executable_hook_becomes_owner_executable_locally_and_on_the_receiver() {
        let root = root("hook-654");
        let hub = MemHub::default();
        let (hook, mac, mini) = executable_hook(&root);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        let source = mac.dir.join(HOOK);
        let dest = mini.dir.join(HOOK);
        assert_eq!(mode_bits(&dest) & 0o100, 0, "receiver started at {:o}", mode_bits(&dest));
        let mtime = std::fs::metadata(&hook).unwrap().modified().unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o654)).unwrap();
        std::fs::File::options().write(true).open(&hook).unwrap().set_modified(mtime).unwrap();
        let before = hub.0.borrow()[HOOK].clone();
        let repaired = sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        assert!(!repaired.pushed.iter().any(|path| path == HOOK), "a mode repair sends no file bytes");
        assert!(!repaired.pulled.iter().any(|path| path == HOOK), "a mode repair fetches no file bytes");
        assert_eq!(mode_bits(&source) & 0o100, 0o100, "local hook mode is {:o}", mode_bits(&source));
        let stored = hub.0.borrow()[HOOK].clone();
        assert_eq!(stored.entry.executable, Some(true));
        assert_eq!(stored.entry.sha, before.entry.sha);
        assert_eq!(stored.data, before.data);
        assert_eq!(stored.entry.modified, before.entry.modified);
        assert_eq!(unpack(stored.data.as_deref().unwrap()).unwrap(), b"#!/bin/sh\necho session-start\n");
        let healed = sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert!(!healed.pulled.iter().any(|path| path == HOOK), "the receiver chmods the hook it has");
        assert_eq!(mode_bits(&dest) & 0o100, 0o100, "receiver hook mode is {:o}", mode_bits(&dest));
        assert_eq!(std::fs::read(&source).unwrap(), b"#!/bin/sh\necho session-start\n");
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho session-start\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_mark_keeps_the_agreed_baseline_and_the_next_sync_pushes_the_local_edit() {
        let root = root("mark-fail");
        let hub = MarkHub { inner: MemHub::default(), fail: std::cell::Cell::new(false) };
        let (hook, mac, _) = executable_hook(&root);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o644)).unwrap();
        let skill = mac.claude_home.join("skills/greet/SKILL.md");
        put(&skill, "one\n");
        let published = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        stamp(&skill, published);
        stamp(&hook, published);
        let mut state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut state, &mac, now).unwrap();
        put(&skill, "two\n");
        stamp(&skill, SystemTime::UNIX_EPOCH + Duration::from_secs(50));
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        hub.fail.set(true);
        let failed = sync_plugins(&hub, &mut state, &mac, now);
        assert_eq!(failed, Err("the hub did not answer within 60 seconds".into()));
        let agreed = saved_agreed(&state, &mac, &root.join("state.json"));
        assert_eq!(agreed.get(USER_SKILL).cloned(), Some(sha(b"one\n")));
        assert_eq!(agreed.get(HOOK).cloned(), Some(sha(b"#!/bin/sh\necho session-start\n")));
        assert_eq!(unpack(hub.inner.0.borrow()[USER_SKILL].data.as_deref().unwrap()).unwrap(), b"one\n");
        hub.fail.set(false);
        let next = sync_plugins(&hub, &mut state, &mac, now).unwrap();
        assert_eq!(next.pushed, vec![USER_SKILL.to_string()]);
        assert_eq!(next.pulled, Vec::<String>::new());
        assert_eq!(std::fs::read(&skill).unwrap(), b"two\n");
        assert_eq!(unpack(hub.inner.0.borrow()[USER_SKILL].data.as_deref().unwrap()).unwrap(), b"two\n");
        assert_eq!(hub.inner.0.borrow()[HOOK].entry.executable, Some(true));
        assert_eq!(unpack(hub.inner.0.borrow()[HOOK].data.as_deref().unwrap()).unwrap(), b"#!/bin/sh\necho session-start\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_chmod_keeps_the_agreed_baseline_and_the_next_sync_pushes_the_local_edit() {
        let root = root("chmod-fail");
        let hub = MemHub::default();
        let (hook, mac, mini) = executable_hook(&root);
        let skill = mac.claude_home.join("skills/greet/SKILL.md");
        put(&skill, "one\n");
        let published = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        stamp(&skill, published);
        stamp(&hook, published);
        let mut mac_state = State::default();
        let mut mini_state = State::default();
        let now = SystemTime::now();
        sync_plugins(&hub, &mut mac_state, &mac, now).unwrap();
        sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        let dest = mini.dir.join(HOOK);
        let mini_skill = mini.claude_home.join("skills/greet/SKILL.md");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644)).unwrap();
        put(&mini_skill, "two\n");
        stamp(&mini_skill, SystemTime::UNIX_EPOCH + Duration::from_secs(50));
        let deny = DenyOwnerExecute::arm();
        let failed = sync_plugins(&hub, &mut mini_state, &mini, now);
        assert_eq!(failed, Err("Operation not permitted (os error 1)".into()));
        let agreed = saved_agreed(&mini_state, &mini, &root.join("state.json"));
        assert_eq!(agreed.get(USER_SKILL).cloned(), Some(sha(b"one\n")));
        assert_eq!(agreed.get(HOOK).cloned(), Some(sha(b"#!/bin/sh\necho session-start\n")));
        drop(deny);
        let next = sync_plugins(&hub, &mut mini_state, &mini, now).unwrap();
        assert_eq!(next.pushed, vec![USER_SKILL.to_string()]);
        assert_eq!(next.pulled, Vec::<String>::new());
        assert_eq!(std::fs::read(&mini_skill).unwrap(), b"two\n");
        assert_eq!(unpack(hub.0.borrow()[USER_SKILL].data.as_deref().unwrap()).unwrap(), b"two\n");
        assert_eq!(mode_bits(&dest) & 0o100, 0o100, "healed hook mode is {:o}", mode_bits(&dest));
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho session-start\n");
        let _ = std::fs::remove_dir_all(root);
    }

    struct OldHub;

    impl Hub for OldHub {
        fn list(&self, _: &str) -> Result<Vec<Entry>, String> {
            Ok(vec![])
        }
        fn push(&self, _: &str, files: Vec<File>) -> Result<(), String> {
            let path = files.first().map(|f| f.entry.path.clone()).unwrap_or_else(|| "plugins/toy@demo/skills/greet/SKILL.md".into());
            Err(format!("{path} is not a file a project shares"))
        }
        fn pull(&self, _: &str, _: Vec<String>) -> Result<Vec<File>, String> {
            Ok(vec![])
        }
        fn mark_executable(&self, _: &str, _: Vec<ModeRepair>) -> Result<(), String> {
            Ok(())
        }
    }
}
