//! The MCP servers Claude Code loads into a session, read from its config files and shown as an
//! agent may see them: name, scope, transport, and what it runs or reaches, never how it signs in.
//! MCP servers that plugins bring are not read.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Where a server is configured, as `claude mcp add --scope` names it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The account's `.claude.json`, for every project.
    User,
    /// The account's `.claude.json`, for one project.
    Local,
    /// The project's `.mcp.json`.
    Project,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Stdio,
    Http,
    Sse,
}

/// One MCP server. Its arguments, environment, headers and the rest of its URL are left out,
/// since they carry its keys.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Server {
    pub name: String,
    pub scope: Scope,
    pub transport: Transport,
    /// A stdio server's program, its file name only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// An http or sse server's URL as scheme, host and port only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// The servers a Claude Code session loads under the account whose home is `claude` (its
/// `CLAUDE_CONFIG_DIR`, or `~/.claude`) in the folder `path`: user scope, then local scope, then
/// project scope, each by name.
pub fn servers(claude: &Path, path: Option<&Path>) -> Vec<Server> {
    let config = config_file(claude).and_then(|file| read_json(&file)).unwrap_or(Value::Null);
    let mut out = listed(&config["mcpServers"], Scope::User);
    if let Some(path) = path {
        let projects = &config["projects"];
        let local = keys_of(path).into_iter().map(|key| &projects[key.as_str()]).find(|project| project.is_object());
        if let Some(project) = local {
            out.extend(listed(&project["mcpServers"], Scope::Local));
        }
        let shared = read_json(&path.join(".mcp.json")).unwrap_or(Value::Null);
        out.extend(listed(&shared["mcpServers"], Scope::Project));
    }
    out
}

/// Claude keeps `.claude.json` inside a `CLAUDE_CONFIG_DIR`, and beside the default `~/.claude`.
fn config_file(claude: &Path) -> Option<PathBuf> {
    [Some(claude.join(".claude.json")), claude.parent().map(|parent| parent.join(".claude.json"))].into_iter().flatten().find(|file| file.is_file())
}

fn read_json(file: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

/// The keys `.claude.json` may file the project at `path` under: as given, and canonical.
fn keys_of(path: &Path) -> Vec<String> {
    let given = path.display().to_string();
    let canonical = path.canonicalize().map(|p| p.display().to_string()).unwrap_or_else(|_| given.clone());
    if canonical == given { vec![given] } else { vec![given, canonical] }
}

fn listed(servers: &Value, scope: Scope) -> Vec<Server> {
    let empty = Map::new();
    let mut out: Vec<Server> = servers.as_object().unwrap_or(&empty).iter().filter_map(|(name, config)| server(name, config, scope)).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn server(name: &str, config: &Value, scope: Scope) -> Option<Server> {
    let command = config["command"].as_str().filter(|c| !c.trim().is_empty());
    let url = config["url"].as_str().filter(|u| !u.trim().is_empty());
    let transport = match config["type"].as_str() {
        Some("stdio") => Transport::Stdio,
        Some("http" | "streamable-http") => Transport::Http,
        Some("sse" | "sse-ide") => Transport::Sse,
        Some(_) => return None,
        None if command.is_some() => Transport::Stdio,
        None if url.is_some() => Transport::Http,
        None => return None,
    };
    let (program, origin) = match transport {
        Transport::Stdio => (command.map(program_of), None),
        Transport::Http | Transport::Sse => (None, url.and_then(origin_of)),
    };
    Some(Server { name: name.to_owned(), scope, transport, program, origin })
}

/// The file name of a command line's program.
fn program_of(command: &str) -> String {
    let program = command.split_whitespace().next().unwrap_or(command);
    program.rsplit(['/', '\\']).next().unwrap_or(program).to_owned()
}

/// `scheme://host[:port]` of a URL, without its user, path, query or fragment.
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or(authority);
    (!scheme.is_empty() && !host.is_empty()).then(|| format!("{}://{host}", scheme.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn write(file: &Path, value: Value) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }

    #[test]
    fn an_accounts_user_local_and_project_servers_show_without_args_env_headers_or_paths() {
        let root = temp("scopes");
        let home = root.join("accounts/work");
        let site = root.join("site");
        let mut config = json!({
            "mcpServers": {
                "linear": { "type": "http", "url": "https://user:pw@mcp.linear.app/mcp?token=lin_api_secret", "headers": { "Authorization": "Bearer lin_api_secret" } },
                "files": { "command": "/opt/tools/bin/files-server --root /secret", "args": ["--token", "abc"], "env": { "API_KEY": "abc" } },
            },
            "projects": { "/elsewhere": { "mcpServers": { "other": { "command": "other" } } } },
        });
        config["projects"][site.display().to_string()] = json!({ "mcpServers": { "db": { "type": "stdio", "command": "npx", "args": ["-y", "@acme/db"], "env": { "DB_PASSWORD": "hunter2" } } } });
        write(&home.join(".claude.json"), config);
        write(&site.join(".mcp.json"), json!({ "mcpServers": { "events": { "type": "sse", "url": "http://localhost:8080/sse" }, "ws": { "type": "ws", "url": "ws://x" } } }));

        let found = servers(&home, Some(&site));
        let shown = serde_json::to_value(&found).unwrap();
        assert_eq!(
            shown,
            json!([
                { "name": "files", "scope": "user", "transport": "stdio", "program": "files-server" },
                { "name": "linear", "scope": "user", "transport": "http", "origin": "https://mcp.linear.app" },
                { "name": "db", "scope": "local", "transport": "stdio", "program": "npx" },
                { "name": "events", "scope": "project", "transport": "sse", "origin": "http://localhost:8080" },
            ])
        );
        for secret in ["lin_api_secret", "hunter2", "abc", "/secret", "@acme/db", "pw@"] {
            assert!(!shown.to_string().contains(secret), "{secret} in {shown}");
        }
        assert_eq!(serde_json::to_value(servers(&home, None)).unwrap().as_array().unwrap().len(), 2, "no folder: user scope only");
    }

    #[test]
    fn the_default_home_reads_the_claude_json_beside_it_and_a_missing_one_lists_nothing() {
        let root = temp("default");
        write(&root.join(".claude.json"), json!({ "mcpServers": { "notes": { "command": "notes" } } }));
        let names: Vec<String> = servers(&root.join(".claude"), None).into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["notes"]);
        assert_eq!(servers(&root.join("nowhere/.claude"), Some(&root.join("nowhere"))), vec![]);
        std::fs::write(root.join("broken.json"), "{").unwrap();
        assert_eq!(read_json(&root.join("broken.json")), None);
    }
}
