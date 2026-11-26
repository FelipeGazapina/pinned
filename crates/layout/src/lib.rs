//! The repo's file layout, checked in code: where a file goes, how it is named, that every file is
//! wired into the build, and which modules and scripts no test reaches.
//!
//! `cargo run -p pinned-layout` prints the whole report and exits 1 on an error. The app's build
//! script and a dev hub run the same check: an error stops them, a file no test reaches only warns.
//! `cargo run -p pinned-layout -- new ui <name>` and `-- new core <name>` create files in place.

mod repo;
mod rust;
pub mod scaffold;
mod ui;

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Finding {
    pub path: PathBuf,
    pub message: String,
}

/// How many files of one kind a test reaches.
pub struct Coverage {
    pub kind: &'static str,
    pub tested: usize,
    pub total: usize,
}

#[derive(Default)]
pub struct Report {
    pub errors: Vec<Finding>,
    pub warnings: Vec<Finding>,
    pub coverage: Vec<Coverage>,
}

/// Checks the checkout at `root`.
pub fn check(root: &Path) -> Report {
    let mut report = Report::default();
    repo::check(root, &mut report);
    for member in rust::members(root) {
        rust::check(root, &member, &mut report);
    }
    ui::check(root, &mut report);
    report
}

impl Report {
    fn error(&mut self, root: &Path, path: &Path, message: impl Into<String>) {
        let path = path.strip_prefix(root).unwrap_or(path).to_owned();
        self.errors.push(Finding { path, message: message.into() });
    }

    fn warn(&mut self, root: &Path, path: &Path, message: impl Into<String>) {
        let path = path.strip_prefix(root).unwrap_or(path).to_owned();
        self.warnings.push(Finding { path, message: message.into() });
    }

    /// One line: how much of the code a test reaches.
    pub fn coverage_line(&self) -> String {
        let parts: Vec<String> = self
            .coverage
            .iter()
            .map(|c| format!("{} of {} {} ({}%)", c.tested, c.total, c.kind, percent(c.tested, c.total)))
            .collect();
        format!("tests reach {}", parts.join(", "))
    }

    pub fn errors_text(&self) -> String {
        self.errors.iter().map(|f| format!("  {}: {}\n", f.path.display(), f.message)).collect()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for w in &self.warnings {
            writeln!(f, "warning: {}: {}", w.path.display(), w.message)?;
        }
        for e in &self.errors {
            writeln!(f, "error: {}: {}", e.path.display(), e.message)?;
        }
        writeln!(f, "{}", self.coverage_line())?;
        write!(f, "{} errors, {} files without tests", self.errors.len(), self.warnings.len())
    }
}

fn percent(part: usize, whole: usize) -> usize {
    (part * 100).checked_div(whole).unwrap_or(100)
}

/// Every file under `dir`, sorted. Hidden entries are skipped.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(dir) = stack.pop() {
        for path in entries(&dir) {
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The entries of `dir`, sorted, hidden ones left out.
fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| !name(p).starts_with('.'))
        .collect();
    out.sort();
    out
}

fn name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Whether `hay` names the file `file`, with no name character right before it, so that `hub.js`
/// is not found inside `github.js`.
fn names(hay: &str, file: &str) -> bool {
    hay.match_indices(file).any(|(at, _)| {
        !hay[..at].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A throwaway checkout holding `files` (path, content).
    pub fn tree(files: &[(&str, &str)]) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir()
            .join(format!("pinned-layout-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&root);
        for (path, content) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        root
    }

    pub fn messages(findings: &[Finding]) -> Vec<String> {
        findings.iter().map(|f| format!("{}: {}", f.path.display(), f.message)).collect()
    }

    #[test]
    fn a_file_name_is_found_only_on_its_own() {
        assert!(names("<script src=\"hub.js\">", "hub.js"));
        assert!(!names("read(\"github.js\")", "hub.js"));
        assert!(!names("gui-hub.js", "hub.js"));
        assert!(names("vendor/hub.js", "hub.js"));
    }

    #[test]
    fn coverage_counts_each_kind() {
        let report = Report {
            coverage: vec![
                Coverage { kind: "Rust modules", tested: 3, total: 4 },
                Coverage { kind: "UI scripts", tested: 0, total: 0 },
            ],
            ..Report::default()
        };
        assert_eq!(report.coverage_line(), "tests reach 3 of 4 Rust modules (75%), 0 of 0 UI scripts (100%)");
    }
}
