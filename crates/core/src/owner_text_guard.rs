//! A guard with no code of its own: its test fails when product code names the hub's first owner
//! or one of their accounts or machines again. Accounts are separate and share no name.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// Words that must not appear in product text, lower case.
    const NEEDLES: [&str; 4] = ["felipe", "gazapina", "bloom", "mac mini"];
    /// The one place the owner's repo name is functional: where the updater looks for a release.
    const RELEASE_URL: &str = "github.com/FelipeGazapina/pinned";

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "tests" && n != "target") {
                    walk(&path, found);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }

    /// The lines of a Rust file that are not test code: up to the top-level `#[cfg(..test..)]` module.
    fn rust_product_lines(text: &str) -> Vec<(usize, &str)> {
        let lines: Vec<&str> = text.lines().collect();
        let mut product = vec![];
        for (i, line) in lines.iter().enumerate() {
            if line.starts_with("#[cfg(") && line.contains("test") {
                let module = lines[i + 1..].iter().find(|l| !l.trim_start().starts_with("#["));
                if module.is_some_and(|m| (m.starts_with("mod ") || m.contains(" mod ")) && m.trim_end().ends_with('{')) {
                    break;
                }
            }
            product.push((i + 1, *line));
        }
        product
    }

    /// A plain comment (not a doc comment, which feeds schema text) says nothing to a user or an agent.
    fn is_plain_rust_comment(line: &str) -> bool {
        let line = line.trim_start();
        line.starts_with("//") && !line.starts_with("///") && !line.starts_with("//!")
    }

    fn is_js_comment(line: &str) -> bool {
        let line = line.trim_start();
        line.starts_with("//") || line.starts_with("/*") || line.starts_with('*')
    }

    fn names_the_owner(line: &str) -> bool {
        let lower = line.to_lowercase();
        NEEDLES.iter().any(|n| lower.contains(n))
    }

    /// The offending `file:line` of every product line in `files` (relative to the repo root).
    fn offenders(files: &[PathBuf], rust: bool) -> Vec<String> {
        let mut out = vec![];
        for file in files {
            let name = file.strip_prefix(root()).unwrap_or(file).to_string_lossy().replace("../", "");
            let text = std::fs::read_to_string(file).unwrap();
            let lines: Vec<(usize, &str)> = if rust { rust_product_lines(&text) } else { text.lines().enumerate().map(|(i, l)| (i + 1, l)).collect() };
            for (n, line) in lines {
                if if rust { is_plain_rust_comment(line) } else { is_js_comment(line) } {
                    continue;
                }
                let line = if name.ends_with("crates/server/src/self_update.rs") { line.replace(RELEASE_URL, "") } else { line.to_owned() };
                if names_the_owner(&line) {
                    out.push(format!("{name}:{n}: {}", line.trim().chars().take(100).collect::<String>()));
                }
            }
        }
        out
    }

    fn product_files() -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut rust = vec![];
        for dir in ["crates/core/src", "crates/server/src", "src"] {
            walk(&root().join(dir), &mut rust);
        }
        let mut js: Vec<PathBuf> = std::fs::read_dir(root().join("ui")).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "js") && !p.to_string_lossy().ends_with(".test.js")).collect();
        js.sort();
        (rust, js)
    }

    #[test]
    fn product_code_names_neither_the_owner_nor_their_accounts_and_machines() {
        let (rust, js) = product_files();
        assert!(rust.len() > 50 && js.len() > 20, "the scan found {} Rust and {} JS files", rust.len(), js.len());
        let mut found = offenders(&rust, true);
        found.extend(offenders(&js, false));
        assert!(found.is_empty(), "product text names the owner or one of their accounts or machines; use the user, or an invented example such as work or the build server:\n{}", found.join("\n"));
    }

    #[test]
    fn the_guard_sees_a_planted_line_and_skips_comments_test_code_and_the_release_url() {
        let text = "let a = \"x\";\n// plain comment about Felipe\n/// doc about the Bloom account\nlet b = \"Mac Mini\";\n#[cfg(test)]\nmod tests {\n    let c = \"felipe\";\n}\n";
        let product: Vec<_> = rust_product_lines(text).into_iter().filter(|(_, l)| !is_plain_rust_comment(l) && names_the_owner(l)).collect();
        assert_eq!(product, [(3, "/// doc about the Bloom account"), (4, "let b = \"Mac Mini\";")]);
        assert!(is_js_comment("  // Felipe") && is_js_comment(" * bloom") && !is_js_comment("let x = 'bloom';"));
        assert!(!names_the_owner(&"const RELEASE_BASE: &str = \"https://github.com/FelipeGazapina/pinned/releases\";".replace(RELEASE_URL, "")));
    }
}
