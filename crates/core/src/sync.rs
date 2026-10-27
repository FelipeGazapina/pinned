//! A project's Claude memory and sessions, its skills git doesn't carry, and pinned's history of
//! its turns, kept the same on every machine of the account through the hub.
//!
//! A project is named by its git origin, since each machine keeps it in a folder of its own. Every
//! machine compares its files with the hub's and with what both held at its last sync, so a file
//! changed on one side only travels that way, a file deleted on one side is deleted on the other,
//! and a file changed on both keeps the newer copy; the history keeps the turns of both.

use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: String,
    pub sha: String,
    /// Unix milliseconds the file last changed on the machine that sent it.
    pub modified: u64,
    /// Compressed bytes.
    #[serde(default)]
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct File {
    #[serde(flatten)]
    pub entry: Entry,
    /// The contents gzipped, in base64; none for a deleted file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// A project of the account, offered on machines that don't have it yet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
}

/// What a machine remembers between syncs: the files' hashes, to skip hashing unchanged files,
/// and what each checkout and the hub agreed on last.
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct State {
    #[serde(default)]
    hashes: BTreeMap<PathBuf, Hashed>,
    /// By `<key> <checkout path>`, so a project moved to another folder starts over instead of
    /// taking its files for deleted.
    #[serde(default)]
    agreed: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Hashed {
    size: u64,
    modified: u64,
    sha: String,
}

impl State {
    pub fn load(file: &Path) -> State {
        std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
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
}

#[derive(Debug, Default, PartialEq)]
pub struct Synced {
    pub pushed: Vec<String>,
    pub pulled: Vec<String>,
}

/// Claude Code's folder for a project's sessions and memory: the path with every character but
/// letters and digits made a dash.
pub fn claude_dir(home: &Path, checkout: &Path) -> PathBuf {
    let slug: String = checkout.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    home.join(".claude/projects").join(slug)
}

/// Brings one checkout and the hub to the same files.
pub fn sync(hub: &impl Hub, state: &mut State, home: &Path, checkout: &Checkout, now: SystemTime) -> Result<Synced, String> {
    let roots = Roots::new(home, checkout);
    let key = &checkout.project.key;
    let mut local: BTreeMap<String, (String, u64)> = BTreeMap::new();
    for (path, file) in roots.files() {
        if let Some(hashed) = state.hash(&file) {
            local.insert(path, hashed);
        }
    }
    let remote: BTreeMap<String, Entry> = hub.list(key)?.into_iter().map(|e| (e.path.clone(), e)).collect();
    let agreed_key = format!("{key} {}", checkout.path.display());
    let mut agreed = state.agreed.remove(&agreed_key).unwrap_or_default();
    let paths: BTreeSet<String> = local.keys().chain(remote.keys()).chain(agreed.keys()).cloned().collect();
    let (mut push, mut pull, mut merge) = (vec![], vec![], vec![]);
    for path in &paths {
        let mine = local.get(path).map(|(sha, _)| sha.clone());
        let theirs = remote.get(path).map(|e| e.sha.clone()).filter(|sha| !sha.is_empty());
        let before = agreed.get(path).cloned();
        if mine == theirs {
            match mine {
                Some(sha) => agreed.insert(path.clone(), sha),
                None => agreed.remove(path),
            };
        } else if mine == before {
            pull.push(path.clone());
        } else if theirs == before {
            let old = local.get(path).is_some_and(|(_, modified)| millis(now).saturating_sub(*modified) > SESSION_AGE.as_millis() as u64);
            if !(before.is_none() && theirs.is_none() && old && path.starts_with("claude/") && path.ends_with(".jsonl")) {
                push.push(path.clone());
            }
        } else if path.as_str() == HISTORY {
            merge.push(path.clone());
        } else if local.get(path).map_or(0, |(_, m)| *m) >= remote.get(path).map_or(0, |e| e.modified) {
            push.push(path.clone());
        } else {
            pull.push(path.clone());
        }
    }
    let mut synced = Synced::default();
    for path in merge {
        let theirs = hub.pull(key, vec![path.clone()])?.into_iter().next().and_then(|f| f.data).map(|d| unpack(&d)).transpose()?.unwrap_or_default();
        let target = roots.local(&path).ok_or("no history here")?;
        let mine = std::fs::read(&target).unwrap_or_default();
        write(&target, &merge_history(&mine, &theirs), None)?;
        push.push(path);
    }

    let mut batch: Vec<File> = vec![];
    let mut size = 0;
    for path in push {
        let file = match roots.local(&path).filter(|f| f.is_file()) {
            None => File { entry: Entry { path: path.clone(), sha: String::new(), modified: millis(now), size: 0 }, data: None },
            Some(target) => {
                let Some((sha, modified)) = state.hash(&target) else { continue };
                let data = pack(&std::fs::read(&target).map_err(|e| e.to_string())?);
                if data.len() as u64 > FILE_LIMIT {
                    continue;
                }
                File { entry: Entry { path: path.clone(), sha, modified, size: data.len() as u64 }, data: Some(base64::engine::general_purpose::STANDARD.encode(data)) }
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
            let Some(target) = roots.local(&file.entry.path) else { continue };
            match &file.data {
                None => {
                    let _ = std::fs::remove_file(&target);
                    agreed.remove(&file.entry.path);
                }
                Some(data) => {
                    let bytes = unpack(data)?;
                    write(&target, &bytes, Some(file.entry.modified))?;
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
    state.agreed.insert(agreed_key, agreed);
    Ok(synced)
}

/// The folders a checkout shares, and how their files are named on the hub.
struct Roots {
    claude: PathBuf,
    /// None while git tracks the skills, which then travel with the repository.
    skills: Option<PathBuf>,
    history: Option<PathBuf>,
}

impl Roots {
    fn new(home: &Path, checkout: &Checkout) -> Roots {
        let skills = checkout.path.join(".claude/skills");
        Roots {
            claude: claude_dir(home, &checkout.path),
            skills: (!tracked(&checkout.path, ".claude/skills")).then_some(skills),
            history: checkout.history.clone(),
        }
    }

    /// Every shared file here, by its name on the hub.
    fn files(&self) -> Vec<(String, PathBuf)> {
        let mut files = vec![];
        for entry in std::fs::read_dir(&self.claude).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".jsonl") && entry.path().is_file() {
                files.push((format!("claude/{name}"), entry.path()));
            }
        }
        walk(&self.claude.join("memory"), "claude/memory", &mut files);
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
        let (root, rest) = if let Some(rest) = path.strip_prefix("claude/") {
            (&self.claude, rest)
        } else if let Some(rest) = path.strip_prefix("skills/") {
            (self.skills.as_ref()?, rest)
        } else {
            return None;
        };
        let rest = Path::new(rest);
        let inside = rest.components().all(|c| matches!(c, Component::Normal(_))) && rest.components().next().is_some();
        let shared = rest.starts_with("memory") || (rest.components().count() == 1 && rest.extension().is_some_and(|e| e == "jsonl"));
        (inside && (root != &self.claude || shared)).then(|| root.join(rest))
    }
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
    let out = Command::new("git").arg("-C").arg(repo).args(["ls-files", "--", path]).stdin(Stdio::null()).stderr(Stdio::null()).output();
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

fn write(target: &Path, bytes: &[u8], modified: Option<u64>) -> Result<(), String> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(target).map_err(|e| format!("could not write {}: {e}", target.display()))?;
    (&file).write_all(bytes).map_err(|e| e.to_string())?;
    if let Some(ms) = modified {
        let _ = file.set_modified(UNIX_EPOCH + Duration::from_millis(ms));
    }
    Ok(())
}

pub fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn pack(bytes: &[u8]) -> Vec<u8> {
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
            Side { checkout: Checkout { project, history: Some(home.join("history/app.jsonl")), path }, home, state: State::default() }
        }
        fn claude(&self, rest: &str) -> PathBuf {
            claude_dir(&self.home, &self.checkout.path).join(rest)
        }
        fn sync(&mut self, hub: &MemHub) -> Synced {
            sync(hub, &mut self.state, &self.home, &self.checkout, SystemTime::now()).unwrap()
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
        assert_eq!(claude_dir(Path::new("/h"), Path::new("/Users/ana/my.app")), PathBuf::from("/h/.claude/projects/-Users-ana-my-app"));
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
        let roots = Roots::new(&side.home, &side.checkout);
        for path in ["claude/../../.ssh/authorized_keys", "claude/settings.json", "claude/s1/subagents/a.jsonl", "skills/../../x", "/etc/passwd", "other/x"] {
            assert_eq!(roots.local(path), None, "{path}");
        }
        assert_eq!(roots.local("claude/memory/a/b.md"), Some(side.claude("memory/a/b.md")));
        let _ = std::fs::remove_dir_all(root);
    }
}
