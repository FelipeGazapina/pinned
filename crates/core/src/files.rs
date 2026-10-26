use crate::workspace::run;
use std::collections::BTreeSet;
use std::path::Path;

const LIMIT: usize = 20_000;
const SKIP: &[&str] = &["node_modules", "target", "dist", "build", "vendor"];

/// The folder's files and the folders holding them, relative to it, folders ending in `/`: what git
/// tracks or would track, or in a folder outside git, what a walk four folders deep finds.
pub fn list(cwd: &Path, path: Option<&str>) -> Vec<String> {
    let files: Vec<String> = match run(cwd, path, "git", &["ls-files", "-z", "-co", "--exclude-standard"]) {
        Ok(out) => out.split('\0').filter(|f| !f.is_empty()).take(LIMIT).map(str::to_owned).collect(),
        Err(_) => {
            let mut found = vec![];
            walk(cwd, "", 4, &mut found);
            found
        }
    };
    let folders: BTreeSet<String> = files.iter().flat_map(|f| f.match_indices('/').map(|(i, _)| format!("{}/", &f[..i]))).collect();
    folders.into_iter().chain(files).collect()
}

fn walk(dir: &Path, prefix: &str, depth: usize, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if found.len() >= LIMIT || name.starts_with('.') || SKIP.contains(&name.as_str()) {
            continue;
        }
        let rel = format!("{prefix}{name}");
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            if depth > 0 {
                walk(&entry.path(), &format!("{rel}/"), depth - 1, found);
            }
        } else {
            found.push(rel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn write(dir: &Path, rel: &str) {
        std::fs::create_dir_all(dir.join(rel).parent().unwrap()).unwrap();
        std::fs::write(dir.join(rel), "x").unwrap();
    }

    #[test]
    fn lists_what_git_would_track_with_the_folders() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Command::new("git").args(["init", "-q"]).current_dir(dir.path()).stdout(Stdio::null()).status().unwrap().success());
        for rel in ["src/main.rs", "src/ui/app.js", "README.md", "café.md", "target/debug/out", "notes.log"] {
            write(dir.path(), rel);
        }
        std::fs::write(dir.path().join(".gitignore"), "target/\n*.log\n").unwrap();
        assert_eq!(list(dir.path(), None), ["src/", "src/ui/", ".gitignore", "README.md", "café.md", "src/main.rs", "src/ui/app.js"]);
    }

    #[test]
    fn walks_a_folder_outside_git() {
        let dir = tempfile::tempdir().unwrap();
        for rel in ["a/b.txt", "node_modules/x/y.js", ".hidden/z", "top.md"] {
            write(dir.path(), rel);
        }
        assert_eq!(list(dir.path(), None), ["a/", "a/b.txt", "top.md"]);
    }
}
