//! The web UI: a flat `ui/` of kebab-case scripts, styles, pages and tests, assets in a few
//! folders, every script and style loaded by something, and which scripts no test reads.

use crate::{entries, files, name, names, read, Coverage, Report};
use std::path::{Path, PathBuf};

/// The only folders in `ui/`. Everything else sits at `ui/<name>`.
const FOLDERS: &[&str] = &["brand", "icons", "vendor"];

/// What a file at the top of `ui/` may be, by the end of its name.
const KINDS: &[&str] = &[".test.js", ".js", ".css", ".html", ".webmanifest", ".fixture.json"];

pub fn check(root: &Path, report: &mut Report) {
    let ui = root.join("ui");
    if !ui.is_dir() {
        return;
    }
    let mut own = vec![];
    for path in entries(&ui) {
        let file = name(&path);
        if path.is_dir() {
            if !FOLDERS.contains(&file) {
                report.error(root, &path, "ui/ is flat: a script, style or test goes at ui/<name>, assets in ui/brand/, ui/icons/ or ui/vendor/");
            }
            for test in files(&path).iter().filter(|p| name(p).ends_with(".test.js")) {
                report.error(root, test, "a UI test runs only from ui/*.test.js (factory/checks.sh): move it to ui/");
            }
        } else if !is_kebab(file) {
            report.error(root, &path, "UI files are kebab-case: lowercase letters, digits and dashes");
        } else if !KINDS.iter().any(|k| file.ends_with(k)) {
            report.error(root, &path, "ui/ holds scripts (.js), styles (.css), pages (.html), tests (.test.js) and test fixtures (.fixture.json)");
        } else {
            own.push(path);
        }
    }

    let texts: Vec<(&PathBuf, String)> = own.iter().map(|p| (p, read(p))).collect();
    let mut rust = String::new();
    for dir in std::iter::once(root.join("src")).chain(entries(&root.join("crates")).into_iter().map(|c| c.join("src"))) {
        for file in files(&dir).iter().filter(|p| p.extension().is_some_and(|e| e == "rs")) {
            rust.push_str(&read(file));
        }
    }
    let tests: String = texts.iter().filter(|(p, _)| is_test(p)).map(|(_, t)| t.as_str()).collect();
    let (mut tested, mut total) = (0, 0);
    for path in own.iter().filter(|p| !is_test(p)) {
        let file = name(path);
        if !(file.ends_with(".js") || file.ends_with(".css")) {
            continue;
        }
        let loaded = texts.iter().any(|(other, text)| *other != path && !is_test(other) && names(text, file)) || names(&rust, file);
        if !loaded {
            report.error(root, path, "nothing loads this file (index.html, another script, a style or the Rust side): wire it or delete it");
        }
        if file.ends_with(".js") {
            total += 1;
            if names(&tests, file) {
                tested += 1;
            } else {
                report.warn(root, path, "no ui/*.test.js reads this script");
            }
        }
    }
    report.coverage.push(Coverage { kind: "UI scripts", tested, total });
}

fn is_test(path: &Path) -> bool {
    name(path).ends_with(".test.js")
}

fn is_kebab(file: &str) -> bool {
    file.starts_with(|c: char| c.is_ascii_lowercase()) && file.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{messages, tree};

    #[test]
    fn misplaced_misnamed_and_unloaded_ui_files_are_errors_and_untested_scripts_warn() {
        let root = tree(&[
            ("ui/index.html", "<script src=\"gui-a.js\"></script><link href=\"gui-a.css\"><script src=\"gui-b.js\"></script>"),
            ("ui/gui-a.js", ""),
            ("ui/gui-a.css", ""),
            ("ui/gui-b.js", ""),
            ("ui/sw.js", ""),
            ("ui/orphan.js", ""),
            ("ui/a.test.js", "read(\"gui-a.js\"); read(\"orphan.js\");"),
            ("ui/Bad_Name.js", ""),
            ("ui/notes.md", ""),
            ("ui/taste.fixture.json", ""),
            ("ui/vendor/x.js", ""),
            ("ui/vendor/x.test.js", ""),
            ("ui/views/y.js", ""),
            ("src/main.rs", "const SW: &str = \"sw.js\";"),
        ]);
        let mut report = Report::default();
        check(&root, &mut report);
        assert_eq!(
            messages(&report.errors).iter().map(|e| e.split_once(": ").unwrap().0).collect::<Vec<_>>(),
            ["ui/Bad_Name.js", "ui/notes.md", "ui/vendor/x.test.js", "ui/views", "ui/orphan.js"]
        );
        assert_eq!(
            messages(&report.warnings),
            ["ui/gui-b.js: no ui/*.test.js reads this script", "ui/sw.js: no ui/*.test.js reads this script"]
        );
        assert_eq!((report.coverage[0].tested, report.coverage[0].total), (2, 4));
    }
}
