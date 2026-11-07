//! Every folder on this machine that holds skills, and the one way to change a skill in one.

use crate::accounts::{AccountId, Provider};
use crate::commands::{enabled_plugins, front_matter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

/// What the machine knows about where skills can be. Built fresh per call, so a new account or
/// project shows at once.
pub struct Places {
    /// `$HOME`, for `~/.agents/skills` and `~/.cursor`.
    pub home: PathBuf,
    /// Each Claude account and its home.
    pub claude_homes: Vec<(AccountId, PathBuf)>,
    /// Each Codex account and its `CODEX_HOME`.
    pub codex_homes: Vec<(AccountId, PathBuf)>,
    /// The Cursor account. Cursor has one home.
    pub cursor: Option<AccountId>,
    /// Known project folders and tab folders, canonical.
    pub projects: Vec<PathBuf>,
    /// `<config>/synced-plugins`, which holds the plugins synced from other machines.
    pub synced_plugins: Option<PathBuf>,
    /// The plugin compiled into pinned, written per process.
    pub pinned_plugin: PathBuf,
    /// Plugin files as installed, kept before pinned's first edit.
    pub originals: PathBuf,
}

/// Where a copy lives. Editability is the variant.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum Place {
    Writable(Writable),
    Fixed(Fixed),
}

/// Places pinned may write. `write_copy` takes only these.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Writable {
    /// `<home>/skills`, synced across homes and machines as `user-skills/<dir>`.
    ClaudePersonal { home: PathBuf },
    /// `<checkout>/.claude/skills`.
    ClaudeProject { checkout: PathBuf },
    /// An enabled plugin's install folder, replaced by the next plugin update. `loaded` is false
    /// for a plugin from a directory marketplace: Claude Code reads its source folder instead, and
    /// this copy only feeds plugin sync.
    PluginCache { home: PathBuf, plugin: String, version: String, loaded: bool },
    /// A directory marketplace's plugin folder, which the Claude homes in `homes` load. Survives updates.
    PluginSource { marketplace: String, plugin: String, homes: Vec<PathBuf> },
    /// `<CODEX_HOME>/skills`, except `.system`.
    CodexPersonal { home: PathBuf },
    /// `~/.cursor/skills`.
    CursorPersonal,
    /// A plugin under `~/.cursor/plugins`. A `cached` one is replaced by the next plugin update.
    CursorPlugin { plugin: String, cached: bool },
    /// `~/.agents/skills`.
    Agents,
}

/// Places someone else owns. Listed and readable, never written.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Fixed {
    /// What claude.ai syncs into `<home>/skills/synced` and `<home>/plugins/synced`.
    ClaudeAccountSynced { home: PathBuf },
    /// `<config>/synced-plugins/plugins/<plugin>`, which sync rewrites from the machine that enables it.
    SyncedPlugin { plugin: String },
    /// Compiled into pinned.
    Pinned,
    /// `<CODEX_HOME>/skills/.system`.
    CodexSystem { home: PathBuf },
    /// `~/.cursor/skills-cursor`.
    CursorBuiltin,
}

impl Fixed {
    pub fn why(&self) -> &'static str {
        match self {
            Fixed::ClaudeAccountSynced { .. } => "Managed by claude.ai, which rewrites it.",
            Fixed::SyncedPlugin { .. } => "Mirror of a plugin from another machine. Edit it on the machine that installed it.",
            Fixed::Pinned => "Built into pinned. Change plugin/skills in the pinned repository and release.",
            Fixed::CodexSystem { .. } => "Built into Codex.",
            Fixed::CursorBuiltin => "Built into Cursor.",
        }
    }
}

/// What the page may offer for a copy.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "edit", rename_all = "kebab-case")]
pub enum Edit {
    Yes,
    /// Editable, but the next plugin update replaces it.
    LostOnUpdate,
    No { why: String },
}

/// The rows of the page: copies group by scope and key, so a personal `deploy` and a project
/// `deploy` are two rows, and `use-railway` in four agents' folders is one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Scope {
    Personal,
    Project { checkout: PathBuf },
    Plugin { plugin: String },
    Builtin,
}

impl Place {
    pub fn edit(&self) -> Edit {
        match self {
            Place::Fixed(fixed) => Edit::No { why: fixed.why().into() },
            Place::Writable(Writable::PluginCache { .. } | Writable::CursorPlugin { cached: true, .. }) => Edit::LostOnUpdate,
            Place::Writable(_) => Edit::Yes,
        }
    }

    pub fn scope(&self) -> Scope {
        let plugin = |id: &str| Scope::Plugin { plugin: id.split('@').next().unwrap_or(id).to_owned() };
        match self {
            Place::Writable(Writable::ClaudePersonal { .. } | Writable::CodexPersonal { .. } | Writable::CursorPersonal | Writable::Agents) => Scope::Personal,
            Place::Writable(Writable::ClaudeProject { checkout }) => Scope::Project { checkout: checkout.clone() },
            Place::Writable(Writable::PluginCache { plugin: id, .. } | Writable::PluginSource { plugin: id, .. } | Writable::CursorPlugin { plugin: id, .. }) => plugin(id),
            Place::Fixed(Fixed::SyncedPlugin { plugin: id }) => plugin(id),
            Place::Fixed(Fixed::ClaudeAccountSynced { .. } | Fixed::Pinned | Fixed::CodexSystem { .. } | Fixed::CursorBuiltin) => Scope::Builtin,
        }
    }

    /// The sync group whose copies follow each other across homes and machines.
    fn linked(&self, dir_name: &str) -> Option<String> {
        matches!(self, Place::Writable(Writable::ClaudePersonal { .. })).then(|| format!("user-skills/{dir_name}"))
    }
}

/// A folder holding skills as `<dir>/<skill>/SKILL.md`. `dir` is canonical, so two routes to one
/// folder (a symlinked home) compare equal.
struct Root {
    dir: PathBuf,
    place: Place,
    /// `<plugin name>:` inside a plugin, else empty.
    prefix: String,
}

/// A Claude folder laid out as `commands/` and `skills/`, or a bare skills folder.
pub(crate) struct ClaudeRoot {
    pub base: PathBuf,
    pub skills_only: bool,
    pub prefix: String,
    pub place: Place,
}

impl ClaudeRoot {
    pub fn skills(&self) -> PathBuf {
        if self.skills_only {
            self.base.clone()
        } else {
            self.base.join("skills")
        }
    }

    /// Whether a session on this home reads it. The cache copy of a directory-marketplace plugin is not.
    pub fn loaded(&self) -> bool {
        !matches!(self.place, Place::Writable(Writable::PluginCache { loaded: false, .. }))
    }
}

/// The Claude folders a session on `home` loads, in Claude's merge order, plus the unloaded cache
/// copies of directory-marketplace plugins. `commands::list` and `roots` both read this.
pub(crate) fn claude_roots(home: &Path, project: Option<&Path>, plugin_dirs: &[PathBuf]) -> Vec<ClaudeRoot> {
    let root = |base: PathBuf, prefix: String, place| ClaudeRoot { base, skills_only: false, prefix, place };
    let mut out = vec![root(home.to_path_buf(), String::new(), Place::Writable(Writable::ClaudePersonal { home: home.to_path_buf() }))];
    if let Some(project) = project {
        out.push(root(project.join(".claude"), String::new(), Place::Writable(Writable::ClaudeProject { checkout: project.to_path_buf() })));
    }
    for plugin in enabled_plugins(home) {
        let prefix = format!("{}:", plugin.name);
        let cache = |loaded| Place::Writable(Writable::PluginCache { home: home.to_path_buf(), plugin: plugin.id.clone(), version: plugin.version.clone(), loaded });
        match directory_source(home, &plugin.id) {
            Some((marketplace, source)) => {
                out.push(root(source, prefix.clone(), Place::Writable(Writable::PluginSource { marketplace, plugin: plugin.id.clone(), homes: vec![home.to_path_buf()] })));
                out.push(root(plugin.path.clone(), prefix, cache(false)));
            }
            None => out.push(root(plugin.path.clone(), prefix, cache(true))),
        }
    }
    for dir in plugin_dirs {
        let Some(id) = dir.file_name().and_then(|s| s.to_str()) else { continue };
        let Some(name) = id.split('@').next().filter(|name| !name.is_empty()) else { continue };
        out.push(root(dir.clone(), format!("{name}:"), Place::Fixed(Fixed::SyncedPlugin { plugin: id.to_owned() })));
    }
    if let Some(account) = crate::commands::account(home) {
        let synced = Place::Fixed(Fixed::ClaudeAccountSynced { home: home.to_path_buf() });
        out.push(ClaudeRoot { base: home.join("skills/synced").join(&account), skills_only: true, prefix: String::new(), place: synced.clone() });
        for (name, dir) in crate::commands::synced_plugins(&home.join("plugins/synced").join(&account)) {
            out.push(root(dir, format!("{name}:"), synced.clone()));
        }
    }
    out
}

/// The marketplace and source folder of an enabled plugin whose marketplace is a local directory.
fn directory_source(home: &Path, id: &str) -> Option<(String, PathBuf)> {
    let (name, marketplace) = id.split_once('@')?;
    let known = read_json(&home.join("plugins/known_marketplaces.json"));
    let entry = &known[marketplace];
    if entry["source"]["source"] != "directory" {
        return None;
    }
    let dir = PathBuf::from(entry["installLocation"].as_str().or_else(|| entry["source"]["path"].as_str())?);
    let manifest = read_json(&dir.join(".claude-plugin/marketplace.json"));
    let plugin = manifest["plugins"].as_array()?.iter().find(|p| p["name"] == name)?;
    let rel = Path::new(plugin["source"].as_str()?);
    rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)).then(|| (marketplace.to_owned(), dir.join(rel)))
}

fn read_json(file: &Path) -> Value {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// The table of skill folders on a machine. Adding a place means adding a row here.
fn roots(places: &Places) -> Vec<Root> {
    let mut rows: Vec<(PathBuf, Place, String)> = vec![];
    for (_, home) in &places.claude_homes {
        rows.extend(claude_roots(home, None, &[]).into_iter().map(|r| (r.skills(), r.place, r.prefix)));
    }
    for checkout in &places.projects {
        rows.push((checkout.join(".claude/skills"), Place::Writable(Writable::ClaudeProject { checkout: checkout.clone() }), String::new()));
    }
    if let Some(synced) = &places.synced_plugins {
        for dir in children(&synced.join("plugins")) {
            let Some(id) = dir.file_name().and_then(|s| s.to_str()).map(str::to_owned) else { continue };
            let name = id.split('@').next().unwrap_or(&id).to_owned();
            rows.push((dir.join("skills"), Place::Fixed(Fixed::SyncedPlugin { plugin: id }), format!("{name}:")));
        }
    }
    rows.push((places.pinned_plugin.join("skills"), Place::Fixed(Fixed::Pinned), "pinned:".into()));
    for (_, home) in &places.codex_homes {
        rows.push((home.join("skills"), Place::Writable(Writable::CodexPersonal { home: home.clone() }), String::new()));
        rows.push((home.join("skills/.system"), Place::Fixed(Fixed::CodexSystem { home: home.clone() }), String::new()));
    }
    let cursor = places.home.join(".cursor");
    rows.push((cursor.join("skills"), Place::Writable(Writable::CursorPersonal), String::new()));
    rows.push((cursor.join("skills-cursor"), Place::Fixed(Fixed::CursorBuiltin), String::new()));
    for (folder, cached) in [("cache", true), ("local", false)] {
        for plugin in cursor_plugins(&cursor.join("plugins").join(folder), 4) {
            let name = plugin_name(&plugin);
            rows.push((plugin.join("skills"), Place::Writable(Writable::CursorPlugin { plugin: name.clone(), cached }), format!("{name}:")));
        }
    }
    rows.push((places.home.join(".agents/skills"), Place::Writable(Writable::Agents), String::new()));

    let mut out: Vec<Root> = vec![];
    for (dir, place, prefix) in rows {
        let Ok(dir) = dunce::canonicalize(&dir) else { continue };
        if !dir.is_dir() {
            continue;
        }
        match out.iter_mut().find(|r| r.dir == dir) {
            Some(Root { place: Place::Writable(Writable::PluginSource { homes, .. }), .. }) => {
                if let Place::Writable(Writable::PluginSource { homes: more, .. }) = place {
                    homes.extend(more);
                }
            }
            Some(_) => {}
            None => out.push(Root { dir, place, prefix }),
        }
    }
    out
}

fn children(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    out.sort();
    out
}

/// Folders under Cursor's plugin folder that hold a `skills` folder, `depth` levels down at most.
fn cursor_plugins(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = vec![];
    for child in children(dir) {
        if child.join("skills").is_dir() {
            out.push(child);
        } else if depth > 0 {
            out.extend(cursor_plugins(&child, depth - 1));
        }
    }
    out
}

fn plugin_name(dir: &Path) -> String {
    [".cursor-plugin/plugin.json", ".claude-plugin/plugin.json"]
        .iter()
        .find_map(|m| read_json(&dir.join(m))["name"].as_str().map(str::to_owned))
        .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// One skill folder in one place. `P` is `Place` after listing, `Writable` once parsed for a write.
#[derive(Clone)]
struct SkillCopy<P> {
    /// Canonical skill folder.
    dir: PathBuf,
    key: String,
    description: String,
    place: P,
}

impl SkillCopy<Place> {
    /// The only way to get a `SkillCopy<Writable>`.
    fn writable(self) -> Result<SkillCopy<Writable>, Fixed> {
        let SkillCopy { dir, key, description, place } = self;
        match place {
            Place::Writable(place) => Ok(SkillCopy { dir, key, description, place }),
            Place::Fixed(fixed) => Err(fixed),
        }
    }
}

/// Every copy on the machine, and the skill folders with no SKILL.md under writable places.
fn inventory(places: &Places) -> (Vec<SkillCopy<Place>>, Vec<PathBuf>) {
    let (mut copies, mut hollow) = (vec![], vec![]);
    for root in roots(places) {
        let personal = matches!(root.place, Place::Writable(Writable::ClaudePersonal { .. }));
        for dir in children(&root.dir) {
            let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
            if name.starts_with('.') || (personal && name == "synced") {
                continue;
            }
            let Ok(dir) = dunce::canonicalize(&dir) else { continue };
            let skill = dir.join("SKILL.md");
            if !skill.is_file() {
                if matches!(root.place, Place::Writable(_)) {
                    hollow.push(dir);
                }
                continue;
            }
            let text = std::fs::read_to_string(&skill).unwrap_or_default();
            let (meta, _) = front_matter(&text);
            copies.push(SkillCopy {
                key: format!("{}{}", root.prefix, meta("name").unwrap_or(name)),
                description: meta("description").unwrap_or_default(),
                place: root.place.clone(),
                dir,
            });
        }
    }
    (copies, hollow)
}

/// SHA-256 over the sorted relative path and content hash of every regular file in `dir`,
/// leaving out symlinks and the names sync leaves out.
fn tree(dir: &Path) -> String {
    let mut hasher = Sha256::new();
    for (rel, path) in files(dir) {
        let sha = std::fs::read(&path).map(|bytes| crate::sync::sha(&bytes)).unwrap_or_default();
        hasher.update(format!("{rel}\0{sha}\n"));
    }
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Regular files under `dir` by their relative path, sorted.
fn files(dir: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
            if crate::sync::blocked(&name) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if meta.is_dir() {
                walk(&entry.path(), &rel, out);
            } else if meta.is_file() {
                out.push((rel, entry.path()));
            }
        }
    }
    let mut out = vec![];
    walk(dir, "", &mut out);
    out.sort();
    out
}

/// A file inside a skill folder, as the client names it: relative, plain components only, and
/// no name sync leaves out, so every file a save can write is part of the folder's `tree`.
pub struct SkillFile(PathBuf);

impl SkillFile {
    pub fn parse(rel: &str) -> Result<SkillFile, String> {
        let path = Path::new(rel);
        let plain = path.components().all(|c| matches!(c, Component::Normal(part) if part.to_str().is_some_and(|p| !crate::sync::blocked(p))));
        if rel.is_empty() || rel.contains('\\') || !plain {
            return Err(format!("{rel:?} is not a file inside a skill folder"));
        }
        Ok(SkillFile(path.to_path_buf()))
    }

    /// The file under `dir`, refused when any part of the way is a symlink.
    fn in_dir(&self, dir: &Path) -> Result<PathBuf, String> {
        let mut path = dir.to_path_buf();
        for part in self.0.components() {
            path.push(part);
            if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!("{} is a symlink, which pinned does not write through", path.display()));
            }
        }
        Ok(path)
    }
}

/// `Call::Skills`.
#[derive(Serialize, Debug, PartialEq)]
pub struct Listing {
    pub copies: Vec<CopyView>,
    /// Skill folders without SKILL.md in writable places.
    pub hollow: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct CopyView {
    /// What an agent types: `unslop`, `pstack:unslop`.
    pub key: String,
    pub scope: Scope,
    pub description: String,
    /// Canonical folder, the copy's address in `ReadSkill` and `SaveSkill`.
    pub dir: String,
    pub place: Place,
    #[serde(flatten)]
    pub edit: Edit,
    pub tree: String,
    pub linked: Option<String>,
    pub readers: Vec<Reader>,
    /// A plugin copy that differs from its kept original.
    pub edited: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Reader {
    pub provider: Provider,
    pub account: AccountId,
}

/// `Call::ReadSkill`.
#[derive(Serialize, Debug, PartialEq)]
pub struct SkillText {
    pub text: String,
    /// The folder's tree now, the `since` for a save.
    pub tree: String,
    /// Every text file in the folder, relative.
    pub files: Vec<String>,
    /// The file as the plugin installed it, when pinned edited it since.
    pub original: Option<String>,
}

/// One copy a save should write, with the tree the client read it at.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Target {
    pub dir: String,
    pub since: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct CopyOutcome {
    pub dir: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Outcome {
    /// Written, or already held this text. `tree` is the folder's tree now.
    Saved { tree: String },
    /// The folder changed since `since`. Nothing was written.
    Changed { tree: String },
    ReadOnly { why: String },
    /// No listed copy has this folder.
    Missing,
    Failed { error: String },
}

/// What a running tab got after the save.
#[derive(Serialize, Debug, PartialEq)]
pub struct Nudged {
    pub tab: crate::wire::TabId,
    pub title: Option<String>,
    pub nudge: Delivery,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Delivery {
    /// The command went in now.
    Reloaded,
    /// The tab is mid-turn. The command goes in once it is idle.
    AfterTurn,
    /// Codex or Cursor sees it when the session starts again.
    NextStart,
}

/// `Call::SaveSkill`.
#[derive(Serialize, Debug, PartialEq)]
pub struct SaveReport {
    pub copies: Vec<CopyOutcome>,
    pub sessions: Vec<Nudged>,
}

pub fn list(places: &Places) -> Listing {
    let (copies, hollow) = inventory(places);
    let sessions = account_sessions(places);
    let copies = copies
        .into_iter()
        .map(|c| {
            let readers = sessions.iter().filter(|(_, s)| reach(&c.place, &s.session(&c.place)).is_some()).map(|(r, _)| Reader { provider: r.provider, account: r.account.clone() }).collect();
            let edited = c.place.edit() == Edit::LostOnUpdate
                && files(&c.dir).iter().any(|(_, path)| original(&places.originals, path).is_some_and(|kept| std::fs::read_to_string(path).ok() != Some(kept)));
            let linked = c.place.linked(&c.dir.file_name().unwrap_or_default().to_string_lossy());
            CopyView { scope: c.place.scope(), edit: c.place.edit(), dir: c.dir.display().to_string(), tree: tree(&c.dir), key: c.key, description: c.description, place: c.place, linked, readers, edited }
        })
        .collect();
    Listing { copies, hollow: hollow.iter().map(|d| d.display().to_string()).collect() }
}

/// One session per account, standing for any the account could run, to name who reads a copy.
struct AccountSession {
    provider: Provider,
    home: PathBuf,
    plugin_dirs: Vec<PathBuf>,
}

impl AccountSession {
    fn session<'a>(&'a self, place: &'a Place) -> Session<'a> {
        let cwd = match place {
            Place::Writable(Writable::ClaudeProject { checkout }) => checkout.as_path(),
            _ => self.home.as_path(),
        };
        Session { provider: self.provider, home: &self.home, cwd, plugin_dirs: &self.plugin_dirs }
    }
}

fn account_sessions(places: &Places) -> Vec<(Reader, AccountSession)> {
    let mut out = vec![];
    for (account, home) in &places.claude_homes {
        let plugin_dirs = places.synced_plugins.as_ref().map(|dir| crate::sync::plugin_dirs(home, dir)).unwrap_or_default();
        out.push((Reader { provider: Provider::Claude, account: account.clone() }, AccountSession { provider: Provider::Claude, home: home.clone(), plugin_dirs }));
    }
    for (account, home) in &places.codex_homes {
        out.push((Reader { provider: Provider::Codex, account: account.clone() }, AccountSession { provider: Provider::Codex, home: home.clone(), plugin_dirs: vec![] }));
    }
    if let Some(account) = &places.cursor {
        out.push((Reader { provider: Provider::Cursor, account: account.clone() }, AccountSession { provider: Provider::Cursor, home: places.home.clone(), plugin_dirs: vec![] }));
    }
    out
}

/// The listed copy at `dir`, whatever route the client named it by.
fn find(copies: &[SkillCopy<Place>], dir: &str) -> Option<SkillCopy<Place>> {
    let dir = dunce::canonicalize(dir).ok()?;
    copies.iter().find(|c| c.dir == dir).cloned()
}

pub fn read(places: &Places, dir: &str, file: &str) -> Result<SkillText, String> {
    let file = SkillFile::parse(file)?;
    let copy = find(&inventory(places).0, dir).ok_or_else(|| format!("no listed skill folder at {dir}"))?;
    let path = file.in_dir(&copy.dir)?;
    let text = String::from_utf8(std::fs::read(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?).map_err(|_| format!("{} is not text", path.display()))?;
    let files = files(&copy.dir).into_iter().filter(|(_, p)| is_text(p)).map(|(rel, _)| rel).collect();
    let original = original(&places.originals, &path).filter(|o| *o != text);
    Ok(SkillText { text, tree: tree(&copy.dir), files, original })
}

fn is_text(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| std::str::from_utf8(&bytes).is_ok() && !bytes[..bytes.len().min(8192)].contains(&0))
}

/// The kept bytes of a plugin file pinned edited, by the file's path.
fn original(originals: &Path, file: &Path) -> Option<String> {
    std::fs::read_to_string(originals.join(crate::sync::sha(file.to_string_lossy().as_bytes()))).ok()
}

/// What `save` did, before the machine nudges sessions.
pub struct Written {
    pub copies: Vec<CopyOutcome>,
    /// Places of copies whose bytes changed. Empty when every copy already held the text.
    pub touched: Vec<Place>,
}

impl Written {
    pub fn report(self, sessions: Vec<Nudged>) -> SaveReport {
        SaveReport { copies: self.copies, sessions }
    }
}

/// Writes `text` to `file` in each target copy that is listed, writable, and unchanged since its
/// `since`. A copy that already holds `text` is `Saved` without a write, so running it twice
/// changes nothing the second time. Never deletes.
pub fn save(places: &Places, file: &str, text: &str, targets: &[Target]) -> Result<Written, String> {
    let file = SkillFile::parse(file)?;
    let copies = inventory(places).0;
    let mut written = Written { copies: vec![], touched: vec![] };
    for target in targets {
        let outcome = match find(&copies, &target.dir).map(SkillCopy::writable) {
            None => Outcome::Missing,
            Some(Err(fixed)) => Outcome::ReadOnly { why: fixed.why().into() },
            Some(Ok(copy)) => write_copy(&copy, &file, text, &target.since, &places.originals, &mut written.touched),
        };
        written.copies.push(CopyOutcome { dir: target.dir.clone(), outcome });
    }
    Ok(written)
}

/// Writes `text` to `file` in `copy` unless the folder's tree is no longer `since`. The tree is
/// taken under `fsx::hold`, so a sync pull cannot land between the check and the write. Keeps the
/// installed bytes first when the place is replaced on update.
fn write_copy(copy: &SkillCopy<Writable>, file: &SkillFile, text: &str, since: &str, originals: &Path, touched: &mut Vec<Place>) -> Outcome {
    let path = match file.in_dir(&copy.dir) {
        Ok(path) => path,
        Err(error) => return Outcome::Failed { error },
    };
    let _held = crate::fsx::hold();
    let now = tree(&copy.dir);
    if std::fs::read(&path).is_ok_and(|bytes| bytes == text.as_bytes()) {
        return Outcome::Saved { tree: now };
    }
    if now != since {
        return Outcome::Changed { tree: now };
    }
    let replaced_on_update = matches!(copy.place, Writable::PluginCache { .. } | Writable::CursorPlugin { cached: true, .. });
    let kept = originals.join(crate::sync::sha(path.to_string_lossy().as_bytes()));
    let written = match std::fs::read(&path) {
        Ok(bytes) if replaced_on_update && !kept.exists() => crate::fsx::replace(&kept, &bytes, None, None),
        _ => Ok(()),
    }
    .and_then(|()| crate::fsx::replace(&path, text.as_bytes(), None, None));
    match written {
        Ok(()) => {
            touched.push(Place::Writable(copy.place.clone()));
            Outcome::Saved { tree: tree(&copy.dir) }
        }
        Err(error) => Outcome::Failed { error },
    }
}

/// What a session needs after a place it loads changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    /// Claude Code reloads personal, project and plugin skills with `/reload-skills`.
    ReloadSkills,
    NextStart,
}

impl Nudge {
    /// The text typed into the terminal, for the nudges that have one.
    pub fn command(self) -> Option<&'static str> {
        match self {
            Nudge::ReloadSkills => Some("/reload-skills"),
            Nudge::NextStart => None,
        }
    }
}

/// The facts of one running session that decide what it loads.
pub struct Session<'a> {
    pub provider: Provider,
    /// The account's CLI home.
    pub home: &'a Path,
    /// Canonical.
    pub cwd: &'a Path,
    /// The `--plugin-dir`s a Claude tab launched with.
    pub plugin_dirs: &'a [PathBuf],
}

/// Whether `session` loads `place`, and what it needs to see a change there. One function, so
/// the page's readers and the tabs nudged after a save never disagree.
pub fn reach(place: &Place, session: &Session) -> Option<Nudge> {
    use Place::{Fixed as F, Writable as W};
    let same_home = |home: &Path| home == session.home;
    let reached = match session.provider {
        Provider::Claude => match place {
            W(Writable::ClaudePersonal { home }) | F(Fixed::ClaudeAccountSynced { home }) => same_home(home),
            W(Writable::ClaudeProject { checkout }) => session.cwd.starts_with(checkout),
            W(Writable::PluginCache { home, loaded, .. }) => *loaded && same_home(home),
            W(Writable::PluginSource { homes, .. }) => homes.iter().any(|h| same_home(h)),
            F(Fixed::SyncedPlugin { plugin }) => session.plugin_dirs.iter().any(|d| d.file_name().is_some_and(|n| n == plugin.as_str())),
            F(Fixed::Pinned) => true,
            _ => false,
        },
        Provider::Codex => match place {
            W(Writable::CodexPersonal { home }) | F(Fixed::CodexSystem { home }) => same_home(home),
            W(Writable::Agents) => true,
            _ => false,
        },
        Provider::Cursor => matches!(place, W(Writable::CursorPersonal | Writable::CursorPlugin { .. } | Writable::Agents) | F(Fixed::CursorBuiltin)),
    };
    reached.then_some(if session.provider == Provider::Claude { Nudge::ReloadSkills } else { Nudge::NextStart })
}

/// Places a plugin-sync cycle changed, from the hub paths it pulled.
pub fn pulled_by_plugin_sync(share: &crate::sync::PluginShare, pulled: &[String]) -> Vec<Place> {
    let mut out: Vec<Place> = vec![];
    for path in pulled {
        let place = if path.starts_with("user-skills/") {
            Place::Writable(Writable::ClaudePersonal { home: share.claude_home.clone() })
        } else if let Some(id) = path.strip_prefix("plugins/").and_then(|rest| rest.split('/').next()) {
            Place::Fixed(Fixed::SyncedPlugin { plugin: id.to_owned() })
        } else {
            continue;
        };
        if !out.contains(&place) {
            out.push(place);
        }
    }
    out
}

/// The project's skills folder, when a project sync pulled into it.
pub fn pulled_by_project_sync(checkout: &crate::sync::Checkout, pulled: &[String]) -> Vec<Place> {
    let path = dunce::canonicalize(&checkout.path).unwrap_or_else(|_| checkout.path.clone());
    let touched = pulled.iter().any(|p| p.starts_with("skills/"));
    touched.then_some(Place::Writable(Writable::ClaudeProject { checkout: path })).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn put(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn skill(dir: &Path, name: &str) {
        put(&dir.join("SKILL.md"), &format!("---\nname: {name}\ndescription: The {name} skill\n---\nBody of {name}.\n"));
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        places: Places,
    }

    impl Fixture {
        fn dir(&self, rel: &str) -> String {
            self.root.join(rel).display().to_string()
        }
    }

    /// A machine with a copy in every place, under a temp folder standing in for `$HOME`.
    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let (home, claude, bloom) = (root.join("home"), root.join("home/.claude"), root.join("config/accounts/bloom"));
        skill(&claude.join("skills/unslop"), "unslop");
        std::fs::create_dir_all(claude.join("skills/emptied/references")).unwrap();
        put(&home.join(".claude.json"), r#"{"oauthAccount":{"organizationUuid":"org","accountUuid":"me"}}"#);
        skill(&claude.join("skills/synced/org_me/brainstorm"), "brainstorm");
        skill(&claude.join("plugins/cache/mkt/kit/1.0/skills/greet"), "greet");
        let mods = root.join("local-mods");
        put(&mods.join(".claude-plugin/marketplace.json"), r#"{"name":"local-mods","plugins":[{"name":"mods","source":"./mods"}]}"#);
        skill(&mods.join("mods/skills/tidy"), "tidy");
        skill(&claude.join("plugins/cache/local-mods/mods/0.1.0/skills/tidy"), "tidy");
        put(
            &claude.join("plugins/known_marketplaces.json"),
            &json!({"local-mods": {"source": {"source": "directory", "path": mods}, "installLocation": mods}}).to_string(),
        );
        put(
            &claude.join("plugins/installed_plugins.json"),
            &json!({"plugins": {
                "kit@mkt": [{"installPath": claude.join("plugins/cache/mkt/kit/1.0"), "version": "1.0"}],
                "mods@local-mods": [{"installPath": claude.join("plugins/cache/local-mods/mods/0.1.0"), "version": "0.1.0"}],
            }})
            .to_string(),
        );
        put(&claude.join("settings.json"), r#"{"enabledPlugins":{"kit@mkt":true,"mods@local-mods":true}}"#);
        skill(&bloom.join("skills/unslop"), "unslop");
        skill(&root.join("app/.claude/skills/deploy"), "deploy");
        let synced = root.join("config/synced-plugins");
        put(&synced.join("plugins/far@mkt/.claude-plugin/plugin.json"), r#"{"name":"far"}"#);
        skill(&synced.join("plugins/far@mkt/skills/far-skill"), "far-skill");
        skill(&root.join("pinned-plugin/skills/bro"), "bro");
        skill(&home.join(".codex/skills/use-railway"), "use-railway");
        skill(&home.join(".codex/skills/.system/imagegen"), "imagegen");
        skill(&home.join(".cursor/skills/use-railway"), "use-railway");
        skill(&home.join(".cursor/skills-cursor/create-rule"), "create-rule");
        put(&home.join(".cursor/plugins/cache/mkt/cp/1.0/.cursor-plugin/plugin.json"), r#"{"name":"cp"}"#);
        skill(&home.join(".cursor/plugins/cache/mkt/cp/1.0/skills/cp-skill"), "cp-skill");
        skill(&home.join(".agents/skills/use-railway"), "use-railway");
        let places = Places {
            home: home.clone(),
            claude_homes: vec![("claude".into(), claude), ("bloom".into(), bloom)],
            codex_homes: vec![("codex".into(), home.join(".codex"))],
            cursor: Some("cursor".into()),
            projects: vec![root.join("app")],
            synced_plugins: Some(synced),
            pinned_plugin: root.join("pinned-plugin"),
            originals: root.join("config/skill-originals"),
        };
        Fixture { _temp: temp, root, places }
    }

    fn kind(place: &Place) -> String {
        serde_json::to_value(place).unwrap()["kind"].as_str().unwrap().to_owned()
    }

    fn copy<'a>(listing: &'a Listing, dir: &str) -> &'a CopyView {
        listing.copies.iter().find(|c| c.dir == dir).unwrap_or_else(|| panic!("{dir} is not listed"))
    }

    fn target(listing: &Listing, dir: &str) -> Target {
        Target { dir: dir.into(), since: copy(listing, dir).tree.clone() }
    }

    fn body(f: &Fixture, rel: &str) -> String {
        std::fs::read_to_string(f.root.join(rel).join("SKILL.md")).unwrap()
    }

    const WRITABLE: [&str; 8] = [
        "home/.claude/skills/unslop",
        "app/.claude/skills/deploy",
        "home/.claude/plugins/cache/mkt/kit/1.0/skills/greet",
        "local-mods/mods/skills/tidy",
        "home/.codex/skills/use-railway",
        "home/.cursor/skills/use-railway",
        "home/.cursor/plugins/cache/mkt/cp/1.0/skills/cp-skill",
        "home/.agents/skills/use-railway",
    ];

    const FIXED: [&str; 5] = [
        "home/.claude/skills/synced/org_me/brainstorm",
        "config/synced-plugins/plugins/far@mkt/skills/far-skill",
        "pinned-plugin/skills/bro",
        "home/.codex/skills/.system/imagegen",
        "home/.cursor/skills-cursor/create-rule",
    ];

    #[test]
    fn every_place_is_listed_with_its_key_scope_editability_and_readers() {
        let f = fixture();
        let listing = list(&f.places);
        let rows: Vec<(String, String, String, Vec<String>)> = listing
            .copies
            .iter()
            .map(|c| {
                let edit = serde_json::to_value(&c.edit).unwrap()["edit"].as_str().unwrap().to_owned();
                let readers = c.readers.iter().map(|r| r.account.clone()).collect();
                (c.key.clone(), kind(&c.place), edit, readers)
            })
            .collect();
        let row = |key: &str, kind: &str, edit: &str, readers: &[&str]| (key.to_owned(), kind.to_owned(), edit.to_owned(), readers.iter().map(|r| (*r).to_owned()).collect::<Vec<_>>());
        assert_eq!(
            rows,
            [
                row("unslop", "claude-personal", "yes", &["claude"]),
                row("kit:greet", "plugin-cache", "lost-on-update", &["claude"]),
                row("mods:tidy", "plugin-source", "yes", &["claude"]),
                row("mods:tidy", "plugin-cache", "lost-on-update", &[]),
                row("brainstorm", "claude-account-synced", "no", &["claude"]),
                row("unslop", "claude-personal", "yes", &["bloom"]),
                row("deploy", "claude-project", "yes", &["claude", "bloom"]),
                row("far:far-skill", "synced-plugin", "no", &["claude", "bloom"]),
                row("pinned:bro", "pinned", "no", &["claude", "bloom"]),
                row("use-railway", "codex-personal", "yes", &["codex"]),
                row("imagegen", "codex-system", "no", &["codex"]),
                row("use-railway", "cursor-personal", "yes", &["cursor"]),
                row("create-rule", "cursor-builtin", "no", &["cursor"]),
                row("cp:cp-skill", "cursor-plugin", "lost-on-update", &["cursor"]),
                row("use-railway", "agents", "yes", &["codex", "cursor"]),
            ]
        );
        let unslop = copy(&listing, &f.dir("home/.claude/skills/unslop"));
        assert_eq!((unslop.scope.clone(), unslop.linked.as_deref()), (Scope::Personal, Some("user-skills/unslop")));
        assert_eq!(copy(&listing, &f.dir("home/.codex/skills/use-railway")).scope, Scope::Personal);
        assert_eq!(copy(&listing, &f.dir("app/.claude/skills/deploy")).scope, Scope::Project { checkout: f.root.join("app") });
        assert_eq!(copy(&listing, &f.dir("config/synced-plugins/plugins/far@mkt/skills/far-skill")).scope, Scope::Plugin { plugin: "far".into() });
        assert_eq!(copy(&listing, &f.dir("pinned-plugin/skills/bro")).scope, Scope::Builtin);
        assert_eq!(serde_json::to_value(copy(&listing, &f.dir("pinned-plugin/skills/bro"))).unwrap()["why"], Fixed::Pinned.why());
    }

    #[test]
    fn a_local_mods_source_is_listed_beside_its_cache_copy_with_the_same_tree() {
        let f = fixture();
        let listing = list(&f.places);
        let source = copy(&listing, &f.dir("local-mods/mods/skills/tidy"));
        let cache = copy(&listing, &f.dir("home/.claude/plugins/cache/local-mods/mods/0.1.0/skills/tidy"));
        assert_eq!((source.key.as_str(), cache.key.as_str()), ("mods:tidy", "mods:tidy"));
        assert_eq!(source.tree, cache.tree);
        assert_eq!(source.scope, cache.scope);
    }

    #[test]
    fn a_skill_folder_without_its_skill_md_is_reported_hollow() {
        let f = fixture();
        assert_eq!(list(&f.places).hollow, [f.dir("home/.claude/skills/emptied")]);
    }

    #[test]
    fn a_save_writes_every_writable_place_and_again_writes_nothing() {
        let f = fixture();
        let listing = list(&f.places);
        let targets: Vec<Target> = WRITABLE.iter().map(|rel| target(&listing, &f.dir(rel))).collect();
        let text = "---\nname: x\ndescription: New\n---\nNew body.\n";
        let first = save(&f.places, "SKILL.md", text, &targets).unwrap();
        assert!(first.copies.iter().all(|c| matches!(c.outcome, Outcome::Saved { .. })), "{:?}", first.copies);
        assert_eq!(first.touched.len(), WRITABLE.len());
        for rel in WRITABLE {
            assert_eq!(body(&f, rel), text, "{rel}");
        }
        let again = save(&f.places, "SKILL.md", text, &targets).unwrap();
        assert!(again.copies.iter().all(|c| matches!(c.outcome, Outcome::Saved { .. })), "{:?}", again.copies);
        assert_eq!(again.touched, []);
    }

    #[test]
    fn a_copy_changed_since_it_was_read_is_not_written() {
        let f = fixture();
        let listing = list(&f.places);
        for rel in WRITABLE {
            let target = target(&listing, &f.dir(rel));
            put(&f.root.join(rel).join("references/added.md"), "someone else");
            let written = save(&f.places, "SKILL.md", "mine", &[target]).unwrap();
            assert!(matches!(written.copies[0].outcome, Outcome::Changed { .. }), "{rel}: {:?}", written.copies);
            assert!(body(&f, rel).starts_with("---\nname:"), "{rel} was written");
        }
    }

    #[test]
    fn a_fixed_place_is_read_only() {
        let f = fixture();
        let listing = list(&f.places);
        for rel in FIXED {
            let before = body(&f, rel);
            let written = save(&f.places, "SKILL.md", "mine", &[target(&listing, &f.dir(rel))]).unwrap();
            assert!(matches!(&written.copies[0].outcome, Outcome::ReadOnly { why } if !why.is_empty()), "{rel}: {:?}", written.copies);
            assert_eq!(body(&f, rel), before);
        }
    }

    #[test]
    fn a_folder_nobody_lists_is_missing_even_when_it_exists() {
        let f = fixture();
        skill(&f.root.join("elsewhere/x"), "x");
        let written = save(&f.places, "SKILL.md", "mine", &[Target { dir: f.dir("elsewhere/x"), since: String::new() }]).unwrap();
        assert_eq!(written.copies[0].outcome, Outcome::Missing);
        assert!(body(&f, "elsewhere/x").starts_with("---"));
    }

    #[test]
    fn paths_that_leave_the_skill_folder_are_refused() {
        let f = fixture();
        let listing = list(&f.places);
        let dir = f.dir("home/.claude/skills/unslop");
        for file in ["../deploy/SKILL.md", "/etc/passwd", "", "a/../../x", ".env", &format!("{}x", crate::fsx::TEMP_PREFIX)] {
            assert!(save(&f.places, file, "mine", &[target(&listing, &dir)]).is_err(), "{file:?} was accepted");
            assert!(read(&f.places, &dir, file).is_err(), "{file:?} was read");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(f.root.join("app"), f.root.join("home/.claude/skills/unslop/out")).unwrap();
            std::os::unix::fs::symlink(f.root.join("app/.claude/skills/deploy/SKILL.md"), f.root.join("home/.claude/skills/unslop/link.md")).unwrap();
            let listing = list(&f.places);
            for file in ["out/.claude/skills/deploy/SKILL.md", "link.md"] {
                let written = save(&f.places, file, "mine", &[target(&listing, &dir)]).unwrap();
                assert!(matches!(written.copies[0].outcome, Outcome::Failed { .. }), "{file}: {:?}", written.copies);
            }
            assert!(body(&f, "app/.claude/skills/deploy").starts_with("---"));
        }
    }

    #[test]
    fn a_save_never_removes_a_file() {
        let f = fixture();
        let rel = "home/.claude/skills/unslop";
        put(&f.root.join(rel).join("references/notes.md"), "notes");
        let listing = list(&f.places);
        save(&f.places, "references/new.md", "new", &[target(&listing, &f.dir(rel))]).unwrap();
        let names: Vec<String> = files(&f.root.join(rel)).into_iter().map(|(rel, _)| rel).collect();
        assert_eq!(names, ["SKILL.md", "references/new.md", "references/notes.md"]);
    }

    #[test]
    fn a_plugin_cache_edit_keeps_the_installed_text_to_diff_against() {
        let f = fixture();
        let dir = f.dir("home/.claude/plugins/cache/mkt/kit/1.0/skills/greet");
        let installed = body(&f, "home/.claude/plugins/cache/mkt/kit/1.0/skills/greet");
        assert_eq!(read(&f.places, &dir, "SKILL.md").unwrap().original, None);
        assert!(!copy(&list(&f.places), &dir).edited);
        save(&f.places, "SKILL.md", "first edit", &[target(&list(&f.places), &dir)]).unwrap();
        save(&f.places, "SKILL.md", "second edit", &[target(&list(&f.places), &dir)]).unwrap();
        let got = read(&f.places, &dir, "SKILL.md").unwrap();
        assert_eq!((got.text.as_str(), got.original.as_deref()), ("second edit", Some(installed.as_str())));
        assert!(copy(&list(&f.places), &dir).edited);
        save(&f.places, "SKILL.md", &installed, &[target(&list(&f.places), &dir)]).unwrap();
        assert!(!copy(&list(&f.places), &dir).edited);
    }

    #[test]
    fn reading_a_copy_gives_its_tree_and_its_text_files() {
        let f = fixture();
        let rel = "home/.claude/skills/unslop";
        put(&f.root.join(rel).join("references/notes.md"), "notes");
        std::fs::write(f.root.join(rel).join("logo.png"), [0u8, 159, 146, 150]).unwrap();
        let listing = list(&f.places);
        let got = read(&f.places, &f.dir(rel), "SKILL.md").unwrap();
        assert_eq!(got.text, "---\nname: unslop\ndescription: The unslop skill\n---\nBody of unslop.\n");
        assert_eq!(got.tree, copy(&listing, &f.dir(rel)).tree);
        assert_eq!(got.files, ["SKILL.md", "references/notes.md"]);
    }

    #[test]
    fn claude_reloads_skills_and_codex_and_cursor_wait_for_their_next_start() {
        let home = PathBuf::from("/h/.claude");
        let session = |provider, cwd: &'static str| Session { provider, home: &home, cwd: Path::new(cwd), plugin_dirs: &[] };
        let personal = Place::Writable(Writable::ClaudePersonal { home: home.clone() });
        let other = Place::Writable(Writable::ClaudePersonal { home: "/h/bloom".into() });
        let project = Place::Writable(Writable::ClaudeProject { checkout: "/src/app".into() });
        let unloaded = Place::Writable(Writable::PluginCache { home: home.clone(), plugin: "mods@local-mods".into(), version: "1".into(), loaded: false });
        assert_eq!(reach(&personal, &session(Provider::Claude, "/x")), Some(Nudge::ReloadSkills));
        assert_eq!(reach(&other, &session(Provider::Claude, "/x")), None);
        assert_eq!(reach(&project, &session(Provider::Claude, "/src/app/crates")), Some(Nudge::ReloadSkills));
        assert_eq!(reach(&project, &session(Provider::Claude, "/src/other")), None);
        assert_eq!(reach(&unloaded, &session(Provider::Claude, "/x")), None);
        assert_eq!(reach(&Place::Writable(Writable::Agents), &session(Provider::Codex, "/x")), Some(Nudge::NextStart));
        assert_eq!(reach(&Place::Writable(Writable::Agents), &session(Provider::Claude, "/x")), None);
        assert_eq!(reach(&Place::Writable(Writable::CursorPersonal), &session(Provider::Cursor, "/x")), Some(Nudge::NextStart));
        assert_eq!(Nudge::ReloadSkills.command(), Some("/reload-skills"));
    }

    #[test]
    fn a_sync_pull_names_the_places_it_changed() {
        let share = crate::sync::PluginShare { claude_home: "/h/.claude".into(), dir: "/c/synced-plugins".into() };
        let pulled = ["user-skills/a/SKILL.md", "user-skills/b/SKILL.md", "plugins/far@mkt/skills/x/SKILL.md"].map(String::from);
        assert_eq!(
            pulled_by_plugin_sync(&share, &pulled),
            [Place::Writable(Writable::ClaudePersonal { home: "/h/.claude".into() }), Place::Fixed(Fixed::SyncedPlugin { plugin: "far@mkt".into() })]
        );
        let project = crate::sync::Project { key: "k".into(), name: "app".into(), repo: "r".into() };
        let checkout = crate::sync::Checkout { project, path: "/nowhere/app".into(), history: None, claude_home: None };
        assert_eq!(pulled_by_project_sync(&checkout, &["claude/memory/x.md".into()]), []);
        assert_eq!(pulled_by_project_sync(&checkout, &["skills/deploy/SKILL.md".into()]), [Place::Writable(Writable::ClaudeProject { checkout: "/nowhere/app".into() })]);
    }
}
