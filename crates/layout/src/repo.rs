//! The top of the checkout: what may sit there, and no docs or rules folders anywhere.

use crate::{entries, name, Report};
use std::path::Path;

/// Everything that may sit at the top of the checkout, hidden entries aside. App code goes in
/// `src/`, shared code in `crates/`, the web UI in `ui/` and tooling in `scripts/`.
const TOP_LEVEL: &[&str] = &[
    "AGENTS.md",
    "CHANGELOG.md",
    "CLAUDE.md",
    "Cargo.lock",
    "Cargo.toml",
    "Dockerfile",
    "Entitlements.plist",
    "Info.plist",
    "README.md",
    "binaries",
    "build.rs",
    "capabilities",
    "clippy.toml",
    "crates",
    "gen",
    "icons",
    "node_modules",
    "package-lock.json",
    "package.json",
    "plugin",
    "scripts",
    "src",
    "target",
    "tauri.conf.json",
    "tauri.linux.conf.json",
    "ui",
];

/// Folders that hold prose for agents. AGENTS.md rules 1 and 2: a rule goes in AGENTS.md, and the
/// code says the rest.
const PROSE: &[&str] = &["docs", "rules"];

/// Folders the walk for prose folders leaves alone: built, installed or third-party files.
const SKIP: &[&str] = &["target", "node_modules", "vendor", "gen"];

pub fn check(root: &Path, report: &mut Report) {
    for path in entries(root) {
        if !TOP_LEVEL.contains(&name(&path)) {
            report.error(root, &path, "nothing new goes at the top of the repo: app code goes in src/, shared code in crates/<name>/, the UI in ui/, tooling in scripts/");
        }
    }
    let mut stack: Vec<_> = entries(root).into_iter().filter(|p| p.is_dir() && !SKIP.contains(&name(p))).collect();
    while let Some(dir) = stack.pop() {
        if PROSE.contains(&name(&dir)) {
            report.error(root, &dir, "no docs or rules folders: a rule goes in AGENTS.md, everything else the code says (AGENTS.md rules 1 and 2)");
            continue;
        }
        stack.extend(entries(&dir).into_iter().filter(|p| p.is_dir() && !SKIP.contains(&name(p))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{messages, tree};

    #[test]
    fn a_stray_file_at_the_top_or_a_docs_folder_is_an_error() {
        let root = tree(&[
            ("Cargo.toml", ""),
            ("notes.txt", ""),
            (".DS_Store", ""),
            ("crates/core/docs/plan.md", ""),
            ("scripts/rules/a.md", ""),
            ("target/docs/x", ""),
        ]);
        let mut report = Report::default();
        check(&root, &mut report);
        let errors = messages(&report.errors);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors[0].starts_with("notes.txt: nothing new goes at the top"));
        assert!(errors.iter().any(|e| e.starts_with("crates/core/docs: no docs or rules folders")));
        assert!(errors.iter().any(|e| e.starts_with("scripts/rules: no docs or rules folders")));
    }
}
