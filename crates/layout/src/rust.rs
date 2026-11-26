//! Each workspace crate: snake_case files, every `.rs` file under `src/` reached by a `mod`
//! declaration, every other file under `src/` included by the code, no docs in `src/`, and which
//! modules no test reaches.

use crate::{files, name, names, read, Coverage, Report};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The workspace members from the root `Cargo.toml`, as folders.
pub fn members(root: &Path) -> Vec<PathBuf> {
    let manifest = read(&root.join("Cargo.toml"));
    let Some(start) = manifest.find("members") else { return vec![] };
    let list = &manifest[start..];
    let list = &list[list.find('[').map_or(0, |i| i + 1)..list.find(']').unwrap_or(list.len())];
    list.split(',')
        .map(|m| m.trim().trim_matches('"'))
        .filter(|m| !m.is_empty())
        .map(|m| normal(&root.join(m)))
        .collect()
}

pub fn check(root: &Path, krate: &Path, report: &mut Report) {
    let src = krate.join("src");
    if !src.is_dir() {
        return;
    }
    let all = files(&src);
    let rs: Vec<&PathBuf> = all.iter().filter(|p| p.extension().is_some_and(|e| e == "rs")).collect();
    for file in &rs {
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        // A file in src/bin/ names its binary, and binaries are kebab-case.
        let stem = if file.parent() == Some(&src.join("bin")) { stem.replace('-', "_") } else { stem.to_owned() };
        if !is_snake(&stem) {
            report.error(root, file, "Rust files are snake_case (kebab-case for a binary in src/bin/)");
        }
    }

    let reached = reached(&roots(krate));
    for file in &rs {
        if !reached.contains(*file) {
            report.error(root, file, "no `mod` declaration reaches this file, so it never compiles: declare it in its parent module or delete it");
        }
    }

    let mut code: String = rs.iter().map(|f| read(f)).collect();
    code.push_str(&read(&krate.join("build.rs")));
    for file in all.iter().filter(|p| !p.extension().is_some_and(|e| e == "rs")) {
        if file.extension().is_some_and(|e| e == "md") {
            report.error(root, file, "no docs in src/: a rule goes in AGENTS.md, everything else the code says (AGENTS.md rules 1 and 2)");
        } else if !names(&code, name(file)) {
            report.error(root, file, "nothing includes this file (include_str!, include_bytes! or build.rs): use it or delete it");
        }
    }

    let tests: String = files(&krate.join("tests")).iter().filter(|p| p.extension().is_some_and(|e| e == "rs")).map(|f| read(f)).collect();
    let (mut tested, mut total) = (0, 0);
    for file in rs.iter().filter(|f| reached.contains(**f) && !is_test(f, &src)) {
        let text = read(file);
        if only_wiring(&text) {
            continue;
        }
        total += 1;
        if has_tests(&text) || tests.contains(&format!("::{}", module_name(file))) {
            tested += 1;
        } else {
            report.warn(root, file, "no test reaches this module: add a #[cfg(test)] module or a test under tests/");
        }
    }
    let kind = "Rust modules";
    match report.coverage.iter_mut().find(|c| c.kind == kind) {
        Some(c) => {
            c.tested += tested;
            c.total += total;
        }
        None => report.coverage.push(Coverage { kind, tested, total }),
    }
}

fn is_snake(stem: &str) -> bool {
    !stem.is_empty() && stem.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The files cargo compiles on its own: `src/lib.rs`, `src/main.rs`, `src/bin/*.rs` and any
/// `path = "..."` a `[lib]` or `[[bin]]` names.
fn roots(krate: &Path) -> Vec<PathBuf> {
    let src = krate.join("src");
    let mut roots: Vec<PathBuf> = ["lib.rs", "main.rs"].iter().map(|f| src.join(f)).filter(|p| p.is_file()).collect();
    roots.extend(crate::entries(&src.join("bin")).into_iter().filter(|p| p.extension().is_some_and(|e| e == "rs")));
    for line in read(&krate.join("Cargo.toml")).lines() {
        if let Some(path) = line.trim().strip_prefix("path").and_then(|r| quoted(r)) {
            if path.ends_with(".rs") {
                roots.push(krate.join(path));
            }
        }
    }
    roots
}

/// Every file reached from `roots` through `mod` declarations (with or without `#[path]`) and
/// `include!`.
fn reached(roots: &[PathBuf]) -> BTreeSet<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<(PathBuf, bool)> = roots.iter().map(|r| (r.clone(), true)).collect();
    while let Some((file, root)) = stack.pop() {
        let file = normal(&file);
        if !file.is_file() || !seen.insert(file.clone()) {
            continue;
        }
        let dir = file.parent().unwrap_or(Path::new("")).to_owned();
        let owns_dir = root || matches!(name(&file), "lib.rs" | "main.rs" | "mod.rs");
        let children = if owns_dir { dir.clone() } else { dir.join(file.file_stem().unwrap_or_default()) };
        let mut path_attr = None;
        for line in read(&file).lines() {
            let (attrs, rest) = split_attrs(line.trim());
            for attr in attrs {
                if let Some(p) = attr.strip_prefix("path").and_then(quoted) {
                    path_attr = Some(p.to_owned());
                }
            }
            if rest.is_empty() {
                continue;
            }
            if let Some(module) = mod_decl(rest) {
                match path_attr.take() {
                    Some(p) => stack.push((dir.join(p), false)),
                    None => stack.extend(find(&children, module, 3).map(|f| (f, false))),
                }
            } else if let Some(p) = rest.find("include!(").and_then(|at| quoted(&rest[at..])) {
                stack.push((dir.join(p), false));
            }
            path_attr = None;
        }
    }
    seen
}

/// Leading `#[...]` attributes of a line, and what follows them.
fn split_attrs(mut line: &str) -> (Vec<&str>, &str) {
    let mut attrs = vec![];
    while let Some(rest) = line.strip_prefix("#[") {
        let mut depth = 1;
        let Some(end) = rest.char_indices().find_map(|(i, c)| {
            depth += match c {
                '[' => 1,
                ']' => -1,
                _ => 0,
            };
            (depth == 0).then_some(i)
        }) else {
            break;
        };
        attrs.push(rest[..end].trim());
        line = rest[end + 1..].trim();
    }
    (attrs, line)
}

/// The module a `mod name;` line declares; `None` for any other line, an inline `mod name {` included.
fn mod_decl(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("pub").map_or(line, |r| {
        let r = r.trim_start();
        match r.strip_prefix('(') {
            Some(scoped) => scoped.split_once(')').map_or(r, |(_, after)| after),
            None => r,
        }
    });
    let module = rest.trim_start().strip_prefix("mod ")?.trim().strip_suffix(';')?.trim();
    is_snake(module).then_some(module)
}

/// `name.rs` or `name/mod.rs` in `dir`, or in a folder below it for a module declared inside an
/// inline `mod x { ... }`.
fn find(dir: &Path, module: &str, depth: usize) -> Option<PathBuf> {
    let direct = [dir.join(format!("{module}.rs")), dir.join(module).join("mod.rs")];
    if let Some(found) = direct.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    if depth == 0 {
        return None;
    }
    crate::entries(dir).into_iter().filter(|p| p.is_dir()).find_map(|sub| find(&sub, module, depth - 1))
}

/// The first string literal in `s`.
fn quoted(s: &str) -> Option<&str> {
    let start = s.find('"')? + 1;
    let len = s[start..].find('"')?;
    Some(&s[start..start + len])
}

/// `a/./b/../c` as `a/c`, so a file reached two ways counts once.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A test file in its own right: `*_tests.rs`, `tests.rs`, or anything in a `tests` folder.
fn is_test(file: &Path, src: &Path) -> bool {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    stem == "tests" || stem.ends_with("_tests") || file.strip_prefix(src).is_ok_and(|p| p.components().any(|c| c.as_os_str() == "tests"))
}

/// A file that only declares and re-exports modules has nothing of its own to test.
fn only_wiring(text: &str) -> bool {
    text.lines().map(str::trim).all(|l| {
        let (_, l) = split_attrs(l);
        l.is_empty() || l.starts_with("//") || mod_decl(l).is_some() || l.starts_with("use ") || l.starts_with("pub use ")
    })
}

fn has_tests(text: &str) -> bool {
    text.contains("#[test]") || text.contains("::test]") || text.contains("::test(")
}

/// The name a module goes by in a path: the file stem, or the folder of a `mod.rs`.
fn module_name(file: &Path) -> &str {
    let file = if name(file) == "mod.rs" { file.parent().unwrap_or(file) } else { file };
    file.file_stem().and_then(|s| s.to_str()).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{messages, tree};

    fn run(files: &[(&str, &str)]) -> Report {
        let root = tree(files);
        let mut report = Report::default();
        for member in members(&root) {
            check(&root, &member, &mut report);
        }
        report
    }

    #[test]
    fn members_come_from_the_workspace_list() {
        let root = tree(&[("Cargo.toml", "[workspace]\nmembers = [\".\", \"crates/core\",\n \"crates/dev\"]\n")]);
        assert_eq!(members(&root), vec![root.clone(), root.join("crates/core"), root.join("crates/dev")]);
    }

    #[test]
    fn every_way_of_wiring_a_module_reaches_it() {
        let report = run(&[
            ("Cargo.toml", "members = [\"crates/a\"]"),
            ("crates/a/Cargo.toml", "[[bin]]\nname = \"x\"\npath = \"src/x.rs\"\n"),
            ("crates/a/src/lib.rs", "pub mod plain;\n#[cfg(unix)] pub(crate) mod gated;\nmod dir;\n#[path = \"elsewhere.rs\"]\nmod moved;\n"),
            ("crates/a/src/plain.rs", "#[cfg(test)]\nmod tests {\n    mod nested;\n}\ninclude!(\"included.rs\");\n"),
            ("crates/a/src/plain/tests/nested.rs", "#[test] fn t() {}"),
            ("crates/a/src/included.rs", ""),
            ("crates/a/src/gated.rs", "#[test] fn t() {}"),
            ("crates/a/src/dir/mod.rs", "mod leaf;"),
            ("crates/a/src/dir/leaf.rs", "fn f() {}"),
            ("crates/a/src/elsewhere.rs", "fn f() {}"),
            ("crates/a/src/x.rs", "fn main() {}"),
            ("crates/a/src/bin/wire-contract.rs", "fn main() {}"),
            ("crates/a/tests/it.rs", "use a::dir::leaf;"),
        ]);
        assert!(report.errors.is_empty(), "{:?}", messages(&report.errors));
    }

    #[test]
    fn an_unwired_file_a_bad_name_a_stray_asset_and_docs_are_errors() {
        let report = run(&[
            ("Cargo.toml", "members = [\".\"]"),
            ("build.rs", "fn main() { let _ = \"src/helper.swift\"; }"),
            ("src/main.rs", "mod used;\nconst S: &str = include_str!(\"screens/a.txt\");\n"),
            ("src/used.rs", ""),
            ("src/orphan.rs", ""),
            ("src/BadName.rs", ""),
            ("src/screens/a.txt", ""),
            ("src/screens/b.txt", ""),
            ("src/helper.swift", ""),
            ("src/notes.md", ""),
        ]);
        let errors = messages(&report.errors);
        assert_eq!(
            errors.iter().map(|e| e.split_once(": ").unwrap().0).collect::<Vec<_>>(),
            ["src/BadName.rs", "src/BadName.rs", "src/orphan.rs", "src/notes.md", "src/screens/b.txt"],
            "{errors:?}"
        );
    }

    #[test]
    fn a_module_without_tests_only_warns() {
        let report = run(&[
            ("Cargo.toml", "members = [\".\"]"),
            ("src/lib.rs", "pub mod tested;\npub mod untested;\npub mod reached;\npub mod wiring;\n"),
            ("src/tested.rs", "fn f() {}\n#[cfg(test)]\nmod tests {\n    #[tokio::test(flavor = \"multi_thread\")]\n    async fn t() {}\n}\n"),
            ("src/untested.rs", "pub fn f() {}"),
            ("src/reached.rs", "pub fn f() {}"),
            ("src/wiring.rs", "// Just wiring.\npub mod inner;\npub use inner::f;\n"),
            ("src/wiring/inner.rs", "#[test] fn t() {}"),
            ("tests/it.rs", "use x::reached::f;"),
        ]);
        assert!(report.errors.is_empty(), "{:?}", messages(&report.errors));
        assert_eq!(messages(&report.warnings), ["src/untested.rs: no test reaches this module: add a #[cfg(test)] module or a test under tests/"]);
        assert_eq!((report.coverage[0].tested, report.coverage[0].total), (3, 4));
    }
}
