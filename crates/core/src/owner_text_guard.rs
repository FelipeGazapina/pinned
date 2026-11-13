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

    /// Folders that hold no product text: tests and fixtures, build output, and vendored third-party code.
    const SKIPPED_DIRS: [&str; 7] = ["tests", "test", "fixtures", "fixture", "target", "node_modules", "vendor"];

    /// Every file under `dir` with one of `extensions`, in path order, skipping `SKIPPED_DIRS` and test files.
    fn walk(dir: &Path, extensions: &[&str], found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if path.is_dir() {
                if !SKIPPED_DIRS.contains(&name.as_str()) {
                    walk(&path, extensions, found);
                }
            } else if path.extension().is_some_and(|e| extensions.iter().any(|x| e == *x)) && !name.ends_with(".test.js") && !name.contains(".fixture.") {
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

    /// JS and CSS: `//`, `/*` and the `*` lines of a block comment.
    fn is_js_comment(line: &str) -> bool {
        let line = line.trim_start();
        line.starts_with("//") || line.starts_with("/*") || line.starts_with('*')
    }

    /// An HTML comment that opens its line; its text is not shown.
    fn is_html_comment(line: &str) -> bool {
        line.trim_start().starts_with("<!--")
    }

    fn is_shell_comment(line: &str) -> bool {
        line.trim_start().starts_with('#')
    }

    /// Whether `line` of the file at `path` is a comment that no user or agent sees; JSON has none.
    fn is_comment(path: &str, line: &str) -> bool {
        match path.rsplit('.').next().unwrap_or("") {
            "rs" => is_plain_rust_comment(line),
            "js" | "css" => is_js_comment(line),
            "html" => is_html_comment(line) || is_js_comment(line),
            "sh" => is_shell_comment(line),
            _ => false,
        }
    }

    fn names_the_owner(line: &str) -> bool {
        let lower = line.to_lowercase();
        NEEDLES.iter().any(|n| lower.contains(n))
    }

    /// The offending `file:line` of every product line in `files` (relative to the repo root).
    fn offenders(files: &[PathBuf]) -> Vec<String> {
        let mut out = vec![];
        for file in files {
            let name = slashed(file.strip_prefix(root()).unwrap_or(file));
            let text = std::fs::read_to_string(file).unwrap();
            let lines: Vec<(usize, &str)> = if name.ends_with(".rs") { rust_product_lines(&text) } else { text.lines().enumerate().map(|(i, l)| (i + 1, l)).collect() };
            for (n, line) in lines {
                if is_comment(&name, line) {
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

    /// A path with `/` between its parts on every platform, and no `../`.
    fn slashed(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/").replace("../", "")
    }

    /// Every file whose text a user or an agent can see: Rust code, everything served from `ui/` (JS, HTML, CSS,
    /// JSON, the web manifest; not tests, fixtures or vendored libraries) and the install script that prints to a terminal.
    fn product_files() -> Vec<PathBuf> {
        let mut files = vec![];
        for dir in ["crates/core/src", "crates/server/src", "src"] {
            walk(&root().join(dir), &["rs"], &mut files);
        }
        walk(&root().join("ui"), &["js", "html", "css", "json", "webmanifest"], &mut files);
        files.push(root().join("crates/server/src/install.sh"));
        files
    }

    #[test]
    fn product_code_names_neither_the_owner_nor_their_accounts_and_machines() {
        let files = product_files();
        let count = |ext: &str| files.iter().filter(|f| f.extension().is_some_and(|e| e == ext)).count();
        assert!(count("rs") > 50 && count("js") > 20 && count("css") > 10 && count("html") >= 1 && count("sh") == 1, "the scan found {} Rust, {} JS, {} CSS, {} HTML files", count("rs"), count("js"), count("css"), count("html"));
        assert!(files.iter().all(|f| !f.to_string_lossy().contains("vendor") && !f.to_string_lossy().ends_with(".test.js")), "vendored and test files are out of the scan");
        let found = offenders(&files);
        assert!(found.is_empty(), "product text names the owner or one of their accounts or machines; use the user, or an invented example such as work or the build server:\n{}", found.join("\n"));
    }

    #[test]
    fn the_guard_sees_a_planted_line_and_skips_comments_test_code_and_the_release_url() {
        let text = "let a = \"x\";\n// plain comment about Felipe\n/// doc about the Bloom account\nlet b = \"Mac Mini\";\n#[cfg(test)]\nmod tests {\n    let c = \"felipe\";\n}\n";
        let product: Vec<_> = rust_product_lines(text).into_iter().filter(|(_, l)| !is_plain_rust_comment(l) && names_the_owner(l)).collect();
        assert_eq!(product, [(3, "/// doc about the Bloom account"), (4, "let b = \"Mac Mini\";")]);
        assert_eq!(slashed(Path::new("crates/server/src\\self_update.rs")), "crates/server/src/self_update.rs", "a Windows path");
        assert!(is_js_comment("  // Felipe") && is_js_comment(" * bloom") && !is_js_comment("let x = 'bloom';"));
        assert!(is_html_comment("  <!-- Felipe -->") && !is_html_comment("<p>Felipe</p>") && is_shell_comment("# Felipe") && !is_shell_comment("echo Felipe"));
        assert!(!names_the_owner(&"const RELEASE_BASE: &str = \"https://github.com/FelipeGazapina/pinned/releases\";".replace(RELEASE_URL, "")));
    }

    #[test]
    fn the_guard_scans_nested_ui_files_of_every_kind_and_leaves_out_vendored_and_test_files() {
        let dir = tempfile::tempdir().unwrap();
        let ui = dir.path().join("ui");
        for (path, text) in [
            ("ui/sub/page.html", "<!-- Felipe -->\n<p>the Mac Mini</p>\n"),
            ("ui/sub/deep/x.js", "// Felipe\nconst a = 'gazapina';\n"),
            ("ui/sub/style.css", "/* Bloom */\n.a::after { content: 'Bloom'; }\n"),
            ("ui/sub/data.json", "{\"name\": \"felipe\"}\n"),
            ("ui/vendor/lib.js", "const a = 'felipe';\n"),
            ("ui/sub/tests/t.js", "const a = 'felipe';\n"),
            ("ui/sub/x.test.js", "const a = 'felipe';\n"),
            ("ui/sub/taste.fixture.json", "{\"name\": \"felipe\"}\n"),
            ("ui/sub/clean.js", "const a = 'the user';\n"),
        ] {
            let file = dir.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let mut files = vec![];
        walk(&ui, &["js", "html", "css", "json", "webmanifest"], &mut files);
        let names: Vec<String> = files.iter().map(|f| slashed(f.strip_prefix(dir.path()).unwrap())).collect();
        assert_eq!(names, ["ui/sub/clean.js", "ui/sub/data.json", "ui/sub/deep/x.js", "ui/sub/page.html", "ui/sub/style.css"]);
        let prefix = slashed(dir.path());
        let found: Vec<String> = offenders(&files).iter().map(|f| f.replace(&prefix, "")).map(|f| f.splitn(3, ':').take(2).collect::<Vec<_>>().join(":")).collect();
        assert_eq!(found, ["/ui/sub/data.json:1", "/ui/sub/deep/x.js:2", "/ui/sub/page.html:2", "/ui/sub/style.css:2"]);
    }
}
