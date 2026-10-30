use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A slash command claude offers, which the composer's `/` menu lists.
#[derive(Serialize, Debug, PartialEq, schemars::JsonSchema)]
pub struct Command {
    /// What follows the slash, such as `compact` or `pstack:unslop`.
    pub name: String,
    pub description: String,
    /// `builtin`, `user`, `project` or `plugin`.
    pub source: &'static str,
    pub skill: bool,
}

/// Claude Code's own commands, as its `/` menu lists them in 2.1.
const BUILTIN: &[(&str, &str)] = &[
    ("add-dir", "Add a new working directory"),
    ("agents", "Manage agent configurations"),
    ("background", "Send this session to the background and free the terminal"),
    ("branch", "Create a branch of the current conversation at this point"),
    ("btw", "Ask a quick side question without interrupting the main conversation"),
    ("clear", "Start a new session with empty context"),
    ("compact", "Free up context by summarizing the conversation so far"),
    ("config", "Open settings"),
    ("context", "Visualize current context usage as a colored grid"),
    ("copy", "Copy Claude's last response to the clipboard"),
    ("diff", "Toggle the diff panel showing uncommitted changes"),
    ("doctor", "Health-check your setup and fix issues"),
    ("effort", "Set effort level for model usage"),
    ("export", "Export the current conversation to a file or clipboard"),
    ("fast", "Toggle fast mode"),
    ("feedback", "Send feedback to Anthropic or report a bug"),
    ("fork", "Copy this conversation into a new background session"),
    ("goal", "Set a goal Claude checks before stopping"),
    ("help", "Show help and available commands"),
    ("hooks", "View hook configurations for tool events"),
    ("ide", "Manage IDE integrations and show status"),
    ("init", "Initialize a CLAUDE.md file with codebase documentation"),
    ("mcp", "Manage MCP servers"),
    ("memory", "Edit CLAUDE.md files and memory settings"),
    ("model", "Set the AI model for Claude Code"),
    ("output-style", "List output styles or switch to one"),
    ("permissions", "Manage allow and deny tool permission rules"),
    ("plan", "Enable plan mode or view the current session plan"),
    ("plugin", "Manage Claude Code plugins"),
    ("recap", "Generate a one-line session recap now"),
    ("release-notes", "View release notes"),
    ("reload-plugins", "Activate pending plugin changes in the current session"),
    ("reload-skills", "Pick up skills added or changed on disk during this session"),
    ("rename", "Rename the current conversation"),
    ("resume", "Resume a previous conversation"),
    ("review", "Review a pull request"),
    ("rewind", "Restore the code and/or conversation to a previous point"),
    ("security-review", "Complete a security review of the pending changes on the current branch"),
    ("skills", "List available skills"),
    ("status", "Show Claude Code status including version, model and account"),
    ("subtask", "Send a subagent off with your full context"),
    ("tasks", "View and manage everything running in the background"),
    ("theme", "Change the theme"),
    ("usage", "Show session cost, plan usage, and activity stats"),
];

/// The commands claude offers in `cwd` when its home is `claude`: its own, then the user's, the
/// project's, and the enabled plugins', each with its skills, then plugins synced from the
/// account's other machines (`shared`), then those claude.ai syncs for the signed-in account.
/// A name seen before is skipped, as claude shows it once.
pub fn list(claude: &Path, cwd: &Path, shared: &[PathBuf]) -> Vec<Command> {
    let mut out: Vec<Command> = BUILTIN
        .iter()
        .map(|(name, description)| Command { name: (*name).into(), description: (*description).into(), source: "builtin", skill: false })
        .collect();
    add_from(&mut out, claude, "", "user");
    add_from(&mut out, &cwd.join(".claude"), "", "project");
    for (name, root) in plugins(claude) {
        add_from(&mut out, &root, &format!("{name}:"), "plugin");
    }
    for dir in shared {
        let Some(name) = dir.file_name().and_then(|s| s.to_str()).and_then(|id| id.split('@').next()).filter(|name| !name.is_empty()) else { continue };
        add_from(&mut out, dir, &format!("{name}:"), "plugin");
    }
    if let Some(account) = account(claude) {
        add_skills(&mut out, &claude.join("skills/synced").join(&account), "", "user");
        for (name, root) in synced_plugins(&claude.join("plugins/synced").join(&account)) {
            add_from(&mut out, &root, &format!("{name}:"), "plugin");
        }
    }
    out
}

/// claude.ai syncs skills and plugins into a folder per signed-in account, named for its
/// organization and account, so the folders of an account signed in before stay on disk. Claude
/// keeps `.claude.json` inside a CLAUDE_CONFIG_DIR, and beside the default `~/.claude`.
fn account(claude: &Path) -> Option<String> {
    let file = [claude.join(".claude.json"), claude.parent()?.join(".claude.json")].into_iter().find(|f| f.is_file())?;
    let config: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let account = &config["oauthAccount"];
    Some(format!("{}_{}", account["organizationUuid"].as_str()?, account["accountUuid"].as_str()?))
}

/// The synced plugins' names and folders, from the account's manifest. A plugin's later generation
/// has its own folder, `design~g2`, beside the one it replaced.
fn synced_plugins(dir: &Path) -> Vec<(String, PathBuf)> {
    let manifest: Value = std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let Some(plugins) = manifest["plugins"].as_array() else { return vec![] };
    plugins
        .iter()
        .filter_map(|p| {
            let name = p["name"].as_str()?;
            let folder = p["generation"].as_u64().map_or_else(|| name.to_owned(), |g| format!("{name}~g{g}"));
            Some((name.to_owned(), dir.join(folder)))
        })
        .collect()
}

fn add_from(out: &mut Vec<Command>, root: &Path, prefix: &str, source: &'static str) {
    add_commands(out, &root.join("commands"), prefix, source);
    add_skills(out, &root.join("skills"), prefix, source);
}

fn add_commands(out: &mut Vec<Command>, dir: &Path, prefix: &str, source: &'static str) {
    let mut found = vec![];
    for file in files(dir, 3, &|p| p.extension().is_some_and(|e| e == "md")) {
        let Some(stem) = file.file_stem().and_then(|s| s.to_str()) else { continue };
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let (meta, body) = front_matter(&text);
        let description = meta("description").unwrap_or_else(|| body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").trim_start_matches('#').trim().to_owned());
        found.push((stem.to_owned(), description, false));
    }
    add(out, found, prefix, source);
}

/// The skills under `dir`, but not the synced ones in its `synced` folder, which belong to accounts.
fn add_skills(out: &mut Vec<Command>, dir: &Path, prefix: &str, source: &'static str) {
    let mut found = vec![];
    let synced = dir.join("synced");
    for file in files(dir, 3, &|p| p.file_name().is_some_and(|n| n == "SKILL.md") && !p.starts_with(&synced)) {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let (meta, _) = front_matter(&text);
        let Some(name) = meta("name").or_else(|| file.parent()?.file_name()?.to_str().map(str::to_owned)) else { continue };
        found.push((name, meta("description").unwrap_or_default(), true));
    }
    add(out, found, prefix, source);
}

fn add(out: &mut Vec<Command>, mut found: Vec<(String, String, bool)>, prefix: &str, source: &'static str) {
    found.sort();
    for (name, description, skill) in found {
        let name = format!("{prefix}{name}");
        if !out.iter().any(|c| c.name == name) {
            out.push(Command { name, description, source, skill });
        }
    }
}

/// The files under `dir`, `depth` folders down at most, that `want` keeps.
fn files(dir: &Path, depth: usize, want: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out = vec![];
    for path in entries.flatten().map(|e| e.path()) {
        if path.is_dir() {
            if depth > 0 {
                out.extend(files(&path, depth - 1, want));
            }
        } else if want(&path) {
            out.push(path);
        }
    }
    out
}

/// The `key: value` lines of a markdown file's front matter, and the text after it. A folded
/// value (`>` or `|`) takes the indented lines below it.
fn front_matter(text: &str) -> (impl Fn(&str) -> Option<String> + '_, &str) {
    let (head, body) = match text.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---")) {
        Some((head, body)) => (head, body),
        None => ("", text),
    };
    let lookup = move |key: &str| {
        let mut lines = head.lines();
        while let Some(line) = lines.next() {
            let Some(value) = line.strip_prefix(key).and_then(|rest| rest.strip_prefix(':')) else { continue };
            let value = value.trim();
            let value = if matches!(value, ">" | "|" | ">-" | "|-") {
                lines.clone().take_while(|l| l.starts_with(' ')).map(str::trim).collect::<Vec<_>>().join(" ")
            } else {
                value.trim_matches(|c| c == '"' || c == '\'').to_owned()
            };
            return Some(value).filter(|v| !v.is_empty());
        }
        None
    };
    (lookup, body)
}

/// One enabled plugin. `enabledPlugins` is true, and the last install record supplies the folder.
/// Older cache versions sit earlier in that list.
pub(crate) struct EnabledPlugin {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub version: String,
    pub git_commit_sha: String,
}

pub(crate) fn enabled_plugins(claude: &Path) -> Vec<EnabledPlugin> {
    let read = |file: &str| std::fs::read_to_string(claude.join(file)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_default();
    let settings = read("settings.json");
    let installed = read("plugins/installed_plugins.json");
    let Some(plugins) = installed["plugins"].as_object() else { return vec![] };
    plugins
        .iter()
        .filter(|(key, _)| settings["enabledPlugins"][key.as_str()] == true)
        .filter_map(|(key, installs)| {
            let record = installs.as_array()?.last()?;
            let path = record["installPath"].as_str()?;
            let name = key.split('@').next()?.to_owned();
            if name.is_empty() {
                return None;
            }
            Some(EnabledPlugin {
                id: (*key).clone(),
                name,
                path: PathBuf::from(path),
                version: record["version"].as_str().unwrap_or("").to_owned(),
                git_commit_sha: record["gitCommitSha"].as_str().unwrap_or("").to_owned(),
            })
        })
        .collect()
}

fn plugins(claude: &Path) -> Vec<(String, PathBuf)> {
    enabled_plugins(claude).into_iter().map(|plugin| (plugin.name, plugin.path)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn lists_builtins_then_user_project_and_plugin_commands_and_skills() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let claude = home.path().join(".claude");
        write(&claude.join("commands/ship.md"), "---\ndescription: \"Ship it\"\n---\nRun the release.");
        write(&claude.join("commands/tools/lint.md"), "# Lint the code\n\nRun the linter.");
        write(&home.path().join(".claude.json"), r#"{"oauthAccount":{"organizationUuid":"org","accountUuid":"me"}}"#);
        write(&claude.join("skills/synced/org_me/brainstorm/SKILL.md"), "---\nname: brainstorm\ndescription: >\n  Ideas, many\n  of them\n---\n");
        write(&claude.join("skills/synced/org_before/stale/SKILL.md"), "---\nname: stale\n---\n");
        let synced = claude.join("plugins/synced/org_me");
        write(&synced.join("manifest.json"), r#"{"plugins":[{"name":"design","generation":2},{"name":"finance"}]}"#);
        write(&synced.join("design~g2/skills/critique/SKILL.md"), "---\nname: critique\ndescription: Review a design\n---\n");
        write(&synced.join("design/skills/old/SKILL.md"), "---\nname: old\n---\n");
        write(&synced.join("finance/commands/close.md"), "Close the month");
        write(&project.path().join(".claude/commands/deploy.md"), "Deploy to staging");
        write(&project.path().join(".claude/commands/compact.md"), "Mine, not claude's");
        let plugin = home.path().join("plugins/kit");
        write(&plugin.join("skills/unslop/SKILL.md"), "---\nname: unslop\ndescription: Cut AI tells\n---\n");
        write(&home.path().join("plugins/off/commands/hidden.md"), "Off");
        write(&claude.join("settings.json"), r#"{"enabledPlugins":{"kit@market":true,"off@market":false}}"#);
        write(
            &claude.join("plugins/installed_plugins.json"),
            &serde_json::json!({"plugins": {
                "kit@market": [{"installPath": plugin}],
                "off@market": [{"installPath": home.path().join("plugins/off")}],
            }})
            .to_string(),
        );

        let all = list(&claude, project.path(), &[]);
        let mine: Vec<_> = all.iter().filter(|c| c.source != "builtin").map(|c| (c.name.as_str(), c.description.as_str(), c.source, c.skill)).collect();
        assert_eq!(
            mine,
            [
                ("lint", "Lint the code", "user", false),
                ("ship", "Ship it", "user", false),
                ("deploy", "Deploy to staging", "project", false),
                ("kit:unslop", "Cut AI tells", "plugin", true),
                ("brainstorm", "Ideas, many of them", "user", true),
                ("design:critique", "Review a design", "plugin", true),
                ("finance:close", "Close the month", "plugin", false),
            ]
        );
        assert_eq!(all.iter().filter(|c| c.name == "compact").count(), 1, "a project command named like a built-in");
        assert_eq!(all[0].source, "builtin");
    }

    #[test]
    fn a_home_with_no_claude_folder_has_only_the_builtins() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(list(&home.path().join(".claude"), home.path(), &[]).len(), BUILTIN.len());
    }
}
