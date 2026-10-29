//! Folders the user works in, offered when opening a tab.

use crate::accounts::AccountId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Every field but `path` has a default, so a file written by another version still loads.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, schemars::JsonSchema)]
pub struct Project {
    /// Canonicalized, since it identifies the project.
    pub path: PathBuf,
    /// Unix seconds.
    #[serde(default)]
    pub last_opened: u64,
    /// The account its sessions run under; none runs them under the default one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountId>,
    /// What the project's orchestrator and its workers keep across conversations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<Memory>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, schemars::JsonSchema)]
pub struct Memory {
    pub id: u64,
    pub text: String,
    /// Unix seconds.
    pub created: u64,
}

impl Project {
    pub fn remember(&mut self, text: &str, now: u64) -> Result<Memory, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("a memory needs some text".into());
        }
        let id = self.memories.iter().map(|m| m.id).max().map_or(1, |last| last + 1);
        let memory = Memory { id, text: text.to_owned(), created: now };
        self.memories.push(memory.clone());
        Ok(memory)
    }

    pub fn forget(&mut self, id: u64) -> Result<(), String> {
        let before = self.memories.len();
        self.memories.retain(|m| m.id != id);
        if self.memories.len() == before {
            return Err(format!("no memory #{id} in {}", self.path.display()));
        }
        Ok(())
    }
}

/// The memories as a session's system prompt lists them; nothing when there are none.
pub fn render(memories: &[Memory]) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = memories.iter().map(|m| format!("- [{}] {}", m.id, m.text)).collect();
    format!("Project memory, by id:\n{}", lines.join("\n"))
}

pub fn load(file: &Path) -> Vec<Project> {
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Loads `file`, or on the first run with it seeds the folder older versions kept in `last_folder`.
pub fn load_or_migrate(file: &Path, last_folder: &Path) -> Vec<Project> {
    if file.exists() {
        return load(file);
    }
    let Ok(legacy) = std::fs::read_to_string(last_folder) else { return vec![] };
    let path = dunce::canonicalize(&legacy).unwrap_or(legacy.into());
    let projects = vec![Project { path, last_opened: 0, account: None, memories: vec![] }];
    save(file, &projects);
    projects
}

pub fn save(file: &Path, projects: &[Project]) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, serde_json::to_vec_pretty(projects).unwrap());
}

pub fn upsert(projects: &mut Vec<Project>, path: PathBuf, now: u64) {
    match projects.iter_mut().find(|p| p.path == path) {
        Some(p) => p.last_opened = now,
        None => projects.push(Project { path, last_opened: now, account: None, memories: vec![] }),
    }
}

/// The projects whose folder still exists, most recently opened first.
pub fn recent(projects: &[Project]) -> Vec<&Project> {
    let mut found: Vec<&Project> = projects.iter().filter(|p| p.path.is_dir()).collect();
    found.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
    found
}

/// Most entries `browse` lists, so a huge folder can't flood the picker.
const BROWSE_LIMIT: usize = 500;

/// The folder a typed path lists, and its subfolders that match.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, schemars::JsonSchema)]
pub struct Folders {
    pub parent: PathBuf,
    pub entries: Vec<Folder>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, schemars::JsonSchema)]
pub struct Folder {
    pub name: String,
    pub path: PathBuf,
    /// Holds a git repository, so it is likely a project.
    pub repo: bool,
}

/// `path` with a leading `~` as the home folder.
pub fn expand(path: &str, home: &Path) -> PathBuf {
    match path.trim().strip_prefix('~') {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(path.trim()),
    }
}

/// The subfolders matching `typed`, a path as typed so far: `~/pro` lists the folders of `~`
/// starting with `pro`, and `~/projetos/` every folder in it. A path without a leading `/` or `~`
/// starts at `home`. Hidden folders show only once the typed name starts with a dot.
pub fn browse(typed: &str, home: &Path) -> Result<Folders, String> {
    #[cfg(windows)]
    let typed = &typed.replace('\\', "/");
    let typed = typed.trim();
    let (folder, prefix) = match typed.rsplit_once('/') {
        _ if typed == "~" => ("~", ""),
        Some((folder, prefix)) => (if folder.is_empty() { "/" } else { folder }, prefix),
        None => ("", typed),
    };
    let parent = match folder.strip_prefix('~') {
        Some(rest) if rest.trim_start_matches('/').is_empty() => home.to_path_buf(),
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None if folder.starts_with('/') => PathBuf::from(folder),
        None => home.join(folder),
    };
    let prefix = prefix.to_lowercase();
    let read = match std::fs::read_dir(&parent) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Ok(Folders { parent, entries: vec![] }),
        Err(_) => return Err(format!("{} is not a folder on this machine", parent.display())),
    };
    let mut entries: Vec<Folder> = read
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let lower = name.to_lowercase();
            let wanted = lower.starts_with(&prefix) && (prefix.starts_with('.') || !name.starts_with('.')) && path.is_dir();
            wanted.then(|| Folder { repo: path.join(".git").exists(), name, path })
        })
        .collect();
    entries.sort_by_key(|f| f.name.to_lowercase());
    entries.truncate(BROWSE_LIMIT);
    Ok(Folders { parent, entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pinned-projects-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn expand_reads_a_leading_tilde_as_home() {
        let home = Path::new("/home/me");
        assert_eq!(expand("~", home), PathBuf::from("/home/me"));
        assert_eq!(expand(" ~/src/app ", home), PathBuf::from("/home/me/src/app"));
        assert_eq!(expand("/srv/app", home), PathBuf::from("/srv/app"));
    }

    #[test]
    fn browse_lists_the_typed_folder_or_the_subfolders_matching_its_last_name() {
        let home = temp("browse");
        for dir in ["projetos/pinned/.git", "projetos/Pessoal", "projetos/.cache", "Documents"] {
            std::fs::create_dir_all(home.join(dir)).unwrap();
        }
        std::fs::write(home.join("projetos/notes.txt"), "").unwrap();
        let names = |typed: &str| {
            let folders = browse(typed, &home).unwrap();
            (folders.parent, folders.entries.into_iter().map(|f| (f.name, f.repo)).collect::<Vec<_>>())
        };
        let projetos = home.join("projetos");
        assert_eq!(names("~/projetos/"), (projetos.clone(), vec![("Pessoal".into(), false), ("pinned".into(), true)]));
        assert_eq!(names("~/projetos/p"), (projetos.clone(), vec![("Pessoal".into(), false), ("pinned".into(), true)]));
        assert_eq!(names("~/projetos/PI"), (projetos.clone(), vec![("pinned".into(), true)]));
        assert_eq!(names("~/projetos/."), (projetos.clone(), vec![(".cache".into(), false)]));
        assert_eq!(names("projetos/pin"), (projetos.clone(), vec![("pinned".into(), true)]));
        assert_eq!(names("").0, home.clone());
        assert_eq!(names("~/").0.to_str(), home.to_str(), "no trailing slash on the folder listed");
        assert_eq!(names("~").1, vec![("Documents".into(), false), ("projetos".into(), false)]);
        assert_eq!(names(&format!("{}/", projetos.display())).1.len(), 2);
        assert_eq!(browse("~/nowhere/", &home), Err(format!("{} is not a folder on this machine", home.join("nowhere").display())));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn upsert_bumps_a_known_path_instead_of_adding_it_again() {
        let mut projects = vec![];
        upsert(&mut projects, "/a".into(), 10);
        upsert(&mut projects, "/b".into(), 20);
        upsert(&mut projects, "/a".into(), 30);
        assert_eq!(
            projects,
            vec![Project { path: "/a".into(), last_opened: 30, account: None, memories: vec![] }, Project { path: "/b".into(), last_opened: 20, account: None, memories: vec![] }]
        );
    }

    #[test]
    fn recent_lists_existing_folders_newest_first() {
        let dir = temp("recent");
        let (old, new) = (dir.join("old"), dir.join("new"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        let projects = vec![
            Project { path: old.clone(), last_opened: 1, account: None, memories: vec![] },
            Project { path: dir.join("deleted"), last_opened: 3, account: None, memories: vec![] },
            Project { path: new.clone(), last_opened: 2, account: None, memories: vec![] },
        ];
        let paths: Vec<&Path> = recent(&projects).into_iter().map(|p| p.path.as_path()).collect();
        assert_eq!(paths, vec![new.as_path(), old.as_path()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn loads_files_with_unknown_or_missing_fields() {
        let file = temp("fields.json");
        std::fs::write(&file, r#"[{"path": "/a", "last_opened": 5, "color": "red"}, {"path": "/b"}]"#).unwrap();
        assert_eq!(
            load(&file),
            vec![Project { path: "/a".into(), last_opened: 5, account: None, memories: vec![] }, Project { path: "/b".into(), last_opened: 0, account: None, memories: vec![] }]
        );
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn projects_survive_a_round_trip() {
        let file = temp("round-trip.json");
        let projects = vec![Project { path: "/repo".into(), last_opened: 1_760_000_000, account: Some("work".into()), memories: vec![] }];
        save(&file, &projects);
        assert_eq!(load(&file), vec![Project { path: "/repo".into(), last_opened: 1_760_000_000, account: Some("work".into()), memories: vec![] }]);
        let _ = std::fs::remove_file(file);
        assert_eq!(load(Path::new("/nonexistent/projects.json")), vec![]);
    }

    #[test]
    fn the_legacy_last_folder_seeds_the_first_run_only() {
        let (file, legacy) = (temp("migrate.json"), temp("last-folder"));
        std::fs::write(&legacy, "/old/folder").unwrap();
        assert_eq!(load_or_migrate(&file, &legacy), vec![Project { path: "/old/folder".into(), last_opened: 0, account: None, memories: vec![] }]);
        save(&file, &[]);
        assert_eq!(load_or_migrate(&file, &legacy), vec![]);
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_file(legacy);
    }

    #[test]
    fn memories_take_the_next_id_and_are_kept_with_their_project() {
        let mut project = Project { path: "/repo".into(), last_opened: 1, account: None, memories: vec![] };
        assert_eq!(render(&project.memories), "");
        assert_eq!(project.remember("  Use pnpm, not npm. ", 100).unwrap().id, 1);
        assert_eq!(project.remember("Tests run with `cargo test --workspace`.", 101).unwrap().id, 2);
        assert_eq!(project.remember(" ", 102), Err("a memory needs some text".into()));
        assert_eq!(project.forget(1), Ok(()));
        assert_eq!(project.forget(1), Err("no memory #1 in /repo".into()));
        assert_eq!(project.remember("The API lives in crates/server.", 103).unwrap().id, 3);
        assert_eq!(render(&project.memories), "Project memory, by id:\n- [2] Tests run with `cargo test --workspace`.\n- [3] The API lives in crates/server.");

        let file = temp("memories.json");
        save(&file, &[project]);
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(raw[0]["memories"][1], serde_json::json!({"id": 3, "text": "The API lives in crates/server.", "created": 103}));
        assert_eq!(load(&file)[0].memories.iter().map(|m| m.id).collect::<Vec<_>>(), [2, 3]);
        let _ = std::fs::remove_file(file);
    }
}
