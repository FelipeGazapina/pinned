//! `new ui <name>` and `new core <name>`: new files where the layout wants them, already wired in,
//! so the check passes from the first build.

use crate::read;
use std::fs;
use std::path::{Path, PathBuf};

/// A UI surface: `ui/gui-<name>.js`, `ui/gui-<name>.css` and `ui/<name>.test.js`, with the script
/// and the style loaded by `ui/index.html` inside `<!-- <name>: start/end -->` markers.
pub fn ui(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(format!("a UI name is kebab-case, like needs-you, not {name}"));
    }
    let ui = root.join("ui");
    let (script, style, test) = (format!("gui-{name}.js"), format!("gui-{name}.css"), format!("{name}.test.js"));
    let html_path = ui.join("index.html");
    let html = read(&html_path);
    for file in [&script, &style, &test] {
        if ui.join(file).exists() {
            return Err(format!("ui/{file} already exists"));
        }
    }
    let (Some(head), Some(body)) = (html.rfind("</head>"), html.rfind("</body>")) else {
        return Err("ui/index.html has no </head> and </body> to load the new files in".into());
    };
    let style_tag = format!("<!-- {name}: start -->\n<link rel=\"stylesheet\" href=\"{style}\">\n<!-- {name}: end -->\n");
    let script_tag = format!("<!-- {name}: start -->\n<script src=\"{script}\"></script>\n<!-- {name}: end -->\n");
    let html = format!("{}{style_tag}{}{script_tag}{}", &html[..head], &html[head..body], &html[body..]);

    write(&ui.join(&script), &format!("/** {name}: what this surface shows, and when. */\n"))?;
    write(&ui.join(&style), &format!("/* {script}'s styles. */\n"))?;
    write(
        &ui.join(&test),
        &format!(
            r#"import assert from "node:assert/strict";
import {{ readFileSync }} from "node:fs";
import test from "node:test";

const read = (name) => readFileSync(new URL(`./${{name}}`, import.meta.url), "utf8");

test("index.html loads {script} and its styles", () => {{
  const html = read("index.html");
  assert.ok(html.includes('<script src="{script}"></script>'));
  assert.ok(html.includes('<link rel="stylesheet" href="{style}">'));
}});
"#
        ),
    )?;
    write(&html_path, &html)?;
    Ok(vec![ui.join(script), ui.join(style), ui.join(test), html_path])
}

/// A module of the shared core: `crates/core/src/<name>.rs`, declared in `lib.rs` in name order.
pub fn core(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(format!("a module name is snake_case, like plan_store, not {name}"));
    }
    let src = root.join("crates/core/src");
    let file = src.join(format!("{name}.rs"));
    if file.exists() || src.join(name).exists() {
        return Err(format!("crates/core/src/{name} already exists"));
    }
    let lib_path = src.join("lib.rs");
    let lib = read(&lib_path);
    let decl = format!("pub mod {name};");
    let mut lines: Vec<&str> = lib.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.strip_prefix("pub mod ").is_some_and(|m| m.trim_end_matches(';') > name))
        .or_else(|| lines.iter().rposition(|l| l.starts_with("pub mod ")).map(|i| i + 1))
        .unwrap_or(lines.len());
    // An attribute belongs to the declaration below it.
    let at = (0..=at).rev().find(|&i| i == 0 || !lines[i - 1].starts_with("#[")).unwrap_or(at);
    lines.insert(at, &decl);

    write(&file, &format!("//! {name}: what this module owns.\n"))?;
    write(&lib_path, &(lines.join("\n") + "\n"))?;
    Ok(vec![file, lib_path])
}

fn write(path: &Path, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{messages, tree};

    #[test]
    fn a_new_ui_surface_is_wired_into_index_html_and_passes_the_check() {
        let root = tree(&[("ui/index.html", "<head>\n<link rel=\"stylesheet\" href=\"gui.css\">\n</head>\n<body>\n<script src=\"gui.js\"></script>\n</body>\n"), ("ui/gui.js", ""), ("ui/gui.css", "")]);
        ui(&root, "needs-you").unwrap();
        let html = read(&root.join("ui/index.html"));
        assert!(html.contains("<!-- needs-you: start -->\n<link rel=\"stylesheet\" href=\"gui-needs-you.css\">\n<!-- needs-you: end -->\n</head>"));
        assert!(html.contains("<!-- needs-you: start -->\n<script src=\"gui-needs-you.js\"></script>\n<!-- needs-you: end -->\n</body>"));
        let report = crate::check(&root);
        assert!(report.errors.is_empty(), "{:?}", messages(&report.errors));
        assert!(!messages(&report.warnings).iter().any(|w| w.contains("gui-needs-you.js")));
        assert_eq!(ui(&root, "needs-you").unwrap_err(), "ui/gui-needs-you.js already exists");
        assert!(ui(&root, "Needs_You").is_err());
    }

    #[test]
    fn a_new_core_module_is_declared_in_name_order() {
        let root = tree(&[
            ("Cargo.toml", "members = [\"crates/core\"]"),
            ("crates/core/src/lib.rs", "pub mod alpha;\npub mod gamma;\n#[cfg(unix)]\npub mod shell;\n"),
            ("crates/core/src/alpha.rs", ""),
            ("crates/core/src/gamma.rs", ""),
            ("crates/core/src/shell.rs", ""),
        ]);
        core(&root, "beta").unwrap();
        core(&root, "zeta").unwrap();
        core(&root, "sh").unwrap();
        assert_eq!(read(&root.join("crates/core/src/lib.rs")), "pub mod alpha;\npub mod beta;\npub mod gamma;\npub mod sh;\n#[cfg(unix)]\npub mod shell;\npub mod zeta;\n");
        let report = crate::check(&root);
        assert!(report.errors.is_empty(), "{:?}", messages(&report.errors));
        assert_eq!(core(&root, "beta").unwrap_err(), "crates/core/src/beta already exists");
    }
}
