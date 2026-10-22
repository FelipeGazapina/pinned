//! Folders the user works in, offered when opening a tab.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Every field but `path` has a default, so a file written by another version still loads.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Project {
    /// Canonicalized, since it identifies the project.
    pub path: PathBuf,
    /// Unix seconds.
    #[serde(default)]
    pub last_opened: u64,
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
    let path = std::fs::canonicalize(&legacy).unwrap_or(legacy.into());
    let projects = vec![Project { path, last_opened: 0 }];
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
        None => projects.push(Project { path, last_opened: now }),
    }
}

/// The projects whose folder still exists, most recently opened first.
pub fn recent(projects: &[Project]) -> Vec<&Project> {
    let mut found: Vec<&Project> = projects.iter().filter(|p| p.path.is_dir()).collect();
    found.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pinned-projects-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn upsert_bumps_a_known_path_instead_of_adding_it_again() {
        let mut projects = vec![];
        upsert(&mut projects, "/a".into(), 10);
        upsert(&mut projects, "/b".into(), 20);
        upsert(&mut projects, "/a".into(), 30);
        assert_eq!(
            projects,
            vec![Project { path: "/a".into(), last_opened: 30 }, Project { path: "/b".into(), last_opened: 20 }]
        );
    }

    #[test]
    fn recent_lists_existing_folders_newest_first() {
        let dir = temp("recent");
        let (old, new) = (dir.join("old"), dir.join("new"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        let projects = vec![
            Project { path: old.clone(), last_opened: 1 },
            Project { path: dir.join("deleted"), last_opened: 3 },
            Project { path: new.clone(), last_opened: 2 },
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
            vec![Project { path: "/a".into(), last_opened: 5 }, Project { path: "/b".into(), last_opened: 0 }]
        );
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn projects_survive_a_round_trip() {
        let file = temp("round-trip.json");
        let projects = vec![Project { path: "/repo".into(), last_opened: 1_760_000_000 }];
        save(&file, &projects);
        assert_eq!(load(&file), vec![Project { path: "/repo".into(), last_opened: 1_760_000_000 }]);
        let _ = std::fs::remove_file(file);
        assert_eq!(load(Path::new("/nonexistent/projects.json")), vec![]);
    }

    #[test]
    fn the_legacy_last_folder_seeds_the_first_run_only() {
        let (file, legacy) = (temp("migrate.json"), temp("last-folder"));
        std::fs::write(&legacy, "/old/folder").unwrap();
        assert_eq!(load_or_migrate(&file, &legacy), vec![Project { path: "/old/folder".into(), last_opened: 0 }]);
        save(&file, &[]);
        assert_eq!(load_or_migrate(&file, &legacy), vec![]);
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_file(legacy);
    }
}
