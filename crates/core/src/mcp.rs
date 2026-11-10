//! MCP servers: the ones Claude Code loads into a session from its own config files, shown as an
//! agent may see them (name, scope, transport, and what it runs or reaches, never how it signs in),
//! and the account's own list on the hub, which pinned loads into every session it starts. MCP
//! servers that plugins bring are not read.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
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

/// How pinned starts or reaches a server of the account's list, as Claude Code's own config
/// names it. Environment and header values are secrets, so only their names are here.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reach {
    /// A program pinned starts beside the session.
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        /// The environment variables it gets. The user types each value in the app.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env: Vec<String>,
    },
    /// A streamable HTTP endpoint.
    Http {
        url: String,
        /// The headers sent with each request. The user types each value in the app.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        headers: Vec<String>,
    },
    /// A server-sent events endpoint.
    Sse {
        url: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        headers: Vec<String>,
    },
}

impl Reach {
    /// The names whose values are secrets: the environment of a program, the headers of a URL.
    pub fn secrets(&self) -> (&'static str, &[String]) {
        match self {
            Reach::Stdio { env, .. } => ("env", env),
            Reach::Http { headers, .. } | Reach::Sse { headers, .. } => ("headers", headers),
        }
    }
}

/// A server of the account's list, as the app and agents write it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Spec {
    /// What sessions call it, and what its tools are named after: letters, digits, `-` and `_`.
    pub name: String,
    /// The one project it loads in, as its repository (`github.com/owner/repo`); every project
    /// when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub transport: Reach,
}

/// The secret values of a server, by name: never in an answer, a list, an audit row or an error.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, schemars::JsonSchema)]
pub struct Values {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl Values {
    fn of(&self, kind: &str) -> &BTreeMap<String, String> {
        if kind == "env" { &self.env } else { &self.headers }
    }
}

/// A server of the account's list as the hub keeps and lists it: every name of a secret it
/// has a value for, never the value.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Entry {
    pub id: String,
    /// Sessions load it only while it is on.
    pub enabled: bool,
    pub server: Spec,
}

/// An enabled server with its secrets, as the hub hands it to a machine that starts a session.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Load {
    pub server: Spec,
    #[serde(default)]
    pub values: Values,
}

/// What `Call::McpImportable` finds in a Claude account's config: a server with the names of its
/// environment and headers, never their values.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct Found {
    pub scope: Scope,
    pub server: Spec,
}

/// The server name pinned's own MCP server goes by in every session.
pub const OWN: &str = "pinned";

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn valid_env(name: &str) -> bool {
    name.len() <= 128 && name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn valid_header(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

impl Spec {
    /// Refuses a server pinned can't load, or one that carries a secret where it would show.
    pub fn check(&self) -> Result<(), String> {
        let name = &self.name;
        if !valid_name(name) {
            return Err(format!("name the server with letters, digits, - and _ (at most 64), not {name:?}"));
        }
        if name.eq_ignore_ascii_case(OWN) {
            return Err(format!("{OWN} is pinned's own server's name"));
        }
        if self.project.as_deref().is_some_and(|p| p.trim().is_empty() || p.trim() != p) {
            return Err("a project is its repository, as github.com/owner/repo".into());
        }
        let shown: Vec<&str> = match &self.transport {
            Reach::Stdio { command, args, .. } => {
                if command.trim().is_empty() {
                    return Err(format!("{name} runs no command"));
                }
                std::iter::once(command.as_str()).chain(args.iter().map(String::as_str)).collect()
            }
            Reach::Http { url, .. } | Reach::Sse { url, .. } => {
                let scheme = url.split_once("://").map(|(scheme, _)| scheme.to_ascii_lowercase());
                if !matches!(scheme.as_deref(), Some("http" | "https")) || origin_of(url).is_none() {
                    return Err(format!("{name}'s URL must start with http:// or https://"));
                }
                vec![url.as_str()]
            }
        };
        if shown.iter().any(|text| crate::surface::mask_secrets(text) != *text) {
            return Err(format!("{name} holds a key in its command, arguments or URL, which lists show: put it in an environment variable or a header"));
        }
        let (kind, names) = self.transport.secrets();
        let valid = if kind == "env" { valid_env } else { valid_header };
        if let Some(bad) = names.iter().find(|n| !valid(n)) {
            return Err(format!("{name}: {bad:?} is not a valid {} name", if kind == "env" { "environment variable" } else { "header" }));
        }
        let mut seen = std::collections::BTreeSet::new();
        if let Some(twice) = names.iter().find(|n| !seen.insert(n.to_ascii_lowercase())) {
            return Err(format!("{name} names {twice} twice"));
        }
        Ok(())
    }

    /// Refuses a value for a name the server doesn't declare, and a declared name with no value in
    /// `values` or among the `kept` names, which the server already has one for.
    pub fn check_values(&self, values: &Values, kept: &[String]) -> Result<(), String> {
        let name = &self.name;
        let (kind, names) = self.transport.secrets();
        let other = if kind == "env" { &values.headers } else { &values.env };
        let given = values.of(kind);
        if let Some(extra) = other.keys().chain(given.keys().filter(|k| !names.contains(k))).next() {
            return Err(format!("{name} has no {extra} to set"));
        }
        if let Some(missing) = names.iter().find(|n| given.get(*n).is_none_or(|v| v.trim().is_empty()) && !kept.contains(n)) {
            return Err(format!("{name} needs a value for {missing}, which the user types in the app"));
        }
        Ok(())
    }
}

/// The servers of `loads` a session in the project `repo` loads: those for every project and
/// those for `repo`, where one for `repo` replaces one for every project with its name. A name
/// in `taken`, which the session's account already configures itself, stays the account's own.
pub fn for_session<'a>(loads: &'a [Load], repo: Option<&str>, taken: &[String]) -> Vec<&'a Load> {
    let mut picked: BTreeMap<&str, &Load> = BTreeMap::new();
    for load in loads.iter().filter(|l| l.server.project.is_none()) {
        picked.insert(&load.server.name, load);
    }
    for load in loads.iter().filter(|l| l.server.project.is_some() && l.server.project.as_deref() == repo) {
        picked.insert(&load.server.name, load);
    }
    picked.into_iter().filter(|(name, _)| !name.eq_ignore_ascii_case(OWN) && !taken.iter().any(|t| t == name)).map(|(_, load)| load).collect()
}

/// The `--mcp-config` document of `loads`, secrets in place.
pub fn session_config(loads: &[&Load]) -> Value {
    let servers: Map<String, Value> = loads
        .iter()
        .map(|load| {
            let config = match &load.server.transport {
                Reach::Stdio { command, args, env } => {
                    let env: Map<String, Value> = env.iter().filter_map(|n| Some((n.clone(), Value::String(load.values.env.get(n)?.clone())))).collect();
                    json!({ "type": "stdio", "command": command, "args": args, "env": env })
                }
                Reach::Http { url, headers } | Reach::Sse { url, headers } => {
                    let kind = if matches!(load.server.transport, Reach::Http { .. }) { "http" } else { "sse" };
                    let headers: Map<String, Value> = headers.iter().filter_map(|n| Some((n.clone(), Value::String(load.values.headers.get(n)?.clone())))).collect();
                    json!({ "type": kind, "url": url, "headers": headers })
                }
            };
            (load.server.name.clone(), config)
        })
        .collect();
    json!({ "mcpServers": servers })
}

/// A session's own MCP config: a folder only its user reads, with `servers.json` in it, removed
/// with everything in it when this is dropped, which the session's end does.
pub struct SessionFiles {
    dir: tempfile::TempDir,
    pub file: PathBuf,
    /// Each server's name and whether it is a program (else a URL), for Codex's flags.
    servers: Vec<(String, Reach)>,
}

const DIR_PREFIX: &str = "pinned-mcp-";

impl SessionFiles {
    /// Writes the config of `loads`, or nothing when there are none.
    pub fn write(loads: &[&Load]) -> std::io::Result<Option<SessionFiles>> {
        if loads.is_empty() {
            return Ok(None);
        }
        sweep_stale();
        let mut builder = tempfile::Builder::new();
        let prefix = format!("{DIR_PREFIX}{}-", std::process::id());
        builder.prefix(&prefix);
        #[cfg(unix)]
        builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
        let dir = builder.tempdir()?;
        let file = dir.path().join("servers.json");
        let mut open = std::fs::OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
        std::io::Write::write_all(&mut open.open(&file)?, session_config(loads).to_string().as_bytes())?;
        let servers = loads.iter().map(|l| (l.server.name.clone(), l.server.transport.clone())).collect();
        Ok(Some(SessionFiles { dir, file, servers }))
    }

    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// Codex's `-c` flags for these servers. A program runs through `exe mcp-run`, which reads its
    /// environment from the file, and an http server's headers come from `exe mcp-headers`, so
    /// no secret lands in an argument or in Codex's environment, which its shell commands inherit.
    /// Codex speaks streamable HTTP only, so an sse server stays out.
    pub fn codex_flags(&self, exe: &Path) -> Vec<String> {
        let (exe_text, file) = (exe.display().to_string(), self.file.display().to_string());
        let mut flags = vec![];
        let mut set = |name: &str, key: &str, value: String| flags.extend(["-c".to_owned(), format!("mcp_servers.{name}.{key}={value}")]);
        for (name, reach) in &self.servers {
            match reach {
                Reach::Stdio { .. } => {
                    set(name, "command", crate::delegate::toml_str(&exe_text));
                    let args = [RUN_SUBCOMMAND, file.as_str(), name.as_str()].map(crate::delegate::toml_str).join(",");
                    set(name, "args", format!("[{args}]"));
                }
                Reach::Http { url, .. } => {
                    set(name, "url", crate::delegate::toml_str(url));
                    let helper = [exe_text.as_str(), HEADERS_SUBCOMMAND, file.as_str(), name.as_str()].map(shell_quote).join(" ");
                    set(name, "http_headers_helper", crate::delegate::toml_str(&helper));
                }
                Reach::Sse { .. } => {}
            }
        }
        flags
    }
}

/// Removes the session folders a pinned that is no longer running left behind, as after a crash.
fn sweep_stale() {
    #[cfg(unix)]
    {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(pid) = name.strip_prefix(DIR_PREFIX).and_then(|rest| rest.split('-').next()).and_then(|pid| pid.parse::<i32>().ok()) else { continue };
            // SAFETY: signal 0 checks that the process exists and sends nothing.
            let gone = pid != std::process::id() as i32 && unsafe { libc::kill(pid, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if gone {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// `pinned mcp-run <file> <name>`: runs the program of server `name` in a session's MCP config
/// with its environment, for Codex.
pub const RUN_SUBCOMMAND: &str = "mcp-run";
/// `pinned mcp-headers <file> <name>`: prints the headers of server `name` as a JSON object, for
/// Codex's `http_headers_helper`.
pub const HEADERS_SUBCOMMAND: &str = "mcp-headers";

fn configured(args: &[String]) -> Result<Value, String> {
    let [file, name] = args else { return Err("takes a session's MCP config file and a server name".into()) };
    let config = read_json(Path::new(file)).ok_or_else(|| format!("{file} is not a session's MCP config"))?;
    let server = config["mcpServers"][name.as_str()].clone();
    if server.is_null() {
        return Err(format!("{file} has no server {name}"));
    }
    Ok(server)
}

pub fn headers(args: &[String]) -> Result<String, String> {
    let server = configured(args)?;
    Ok(server.get("headers").cloned().unwrap_or_else(|| json!({})).to_string())
}

/// Runs the server's program in place of this process, so Codex talks to it directly.
pub fn run(args: &[String]) -> Result<String, String> {
    let server = configured(args)?;
    let command = server["command"].as_str().ok_or("the server is not a program")?;
    let mut child = crate::process::command(command);
    child.args(server["args"].as_array().into_iter().flatten().filter_map(Value::as_str));
    for (name, value) in server["env"].as_object().into_iter().flatten() {
        child.env(name, value.as_str().unwrap_or_default());
    }
    #[cfg(unix)]
    {
        let error = std::os::unix::process::CommandExt::exec(&mut child);
        Err(format!("could not run {command}: {error}"))
    }
    #[cfg(not(unix))]
    {
        let status = child.status().map_err(|e| format!("could not run {command}: {e}"))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// What can be imported from the account whose home is `claude`, in the folder `path`: each
/// server `servers` lists, with its command, arguments and URL, and the names of its environment
/// and headers.
pub fn importable(claude: &Path, path: Option<&Path>) -> Vec<Found> {
    let config = config_file(claude).and_then(|file| read_json(&file)).unwrap_or(Value::Null);
    let mut out = found(&config["mcpServers"], Scope::User);
    if let Some(path) = path {
        let projects = &config["projects"];
        if let Some(project) = keys_of(path).into_iter().map(|key| &projects[key.as_str()]).find(|project| project.is_object()) {
            out.extend(found(&project["mcpServers"], Scope::Local));
        }
        out.extend(found(&read_json(&path.join(".mcp.json")).unwrap_or(Value::Null)["mcpServers"], Scope::Project));
    }
    out
}

fn found(servers: &Value, scope: Scope) -> Vec<Found> {
    let names = |value: &Value| -> Vec<String> { value.as_object().map(|map| map.keys().cloned().collect()).unwrap_or_default() };
    listed(servers, scope)
        .into_iter()
        .filter_map(|listed| {
            let config = &servers[listed.name.as_str()];
            let transport = match listed.transport {
                Transport::Stdio => Reach::Stdio {
                    command: config["command"].as_str()?.to_owned(),
                    args: config["args"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect(),
                    env: names(&config["env"]),
                },
                Transport::Http => Reach::Http { url: config["url"].as_str()?.to_owned(), headers: names(&config["headers"]) },
                Transport::Sse => Reach::Sse { url: config["url"].as_str()?.to_owned(), headers: names(&config["headers"]) },
            };
            Some(Found { scope, server: Spec { name: listed.name, project: None, transport } })
        })
        .collect()
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

    fn stdio(name: &str, project: Option<&str>, env: &[&str]) -> Spec {
        Spec { name: name.into(), project: project.map(Into::into), transport: Reach::Stdio { command: "npx".into(), args: vec!["-y".into(), format!("@acme/{name}")], env: env.iter().map(|e| e.to_string()).collect() } }
    }

    fn http(name: &str, headers: &[&str]) -> Spec {
        Spec { name: name.into(), project: None, transport: Reach::Http { url: format!("https://{name}.example.com/mcp"), headers: headers.iter().map(|h| h.to_string()).collect() } }
    }

    fn values(env: &[(&str, &str)], headers: &[(&str, &str)]) -> Values {
        let map = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Values { env: map(env), headers: map(headers) }
    }

    #[test]
    fn the_wire_shape_follows_claudes_own_config_with_names_for_secrets() {
        let spec = stdio("ctx", Some("github.com/acme/site"), &["API_KEY"]);
        assert_eq!(serde_json::to_value(&spec).unwrap(), json!({ "name": "ctx", "project": "github.com/acme/site", "transport": { "type": "stdio", "command": "npx", "args": ["-y", "@acme/ctx"], "env": ["API_KEY"] } }));
        let parsed: Spec = serde_json::from_value(json!({ "name": "lin", "transport": { "type": "http", "url": "https://mcp.linear.app/mcp" } })).unwrap();
        assert_eq!(parsed, Spec { name: "lin".into(), project: None, transport: Reach::Http { url: "https://mcp.linear.app/mcp".into(), headers: vec![] } });
        assert!(serde_json::from_value::<Spec>(json!({ "name": "x", "transport": { "type": "stdio", "command": "a", "env": { "API_KEY": "v" } } })).is_err(), "env takes names, not values");
    }

    #[test]
    fn a_server_pinned_cant_load_or_that_shows_a_key_is_refused() {
        assert_eq!(stdio("ctx", None, &["API_KEY", "_X1"]).check(), Ok(()));
        assert_eq!(http("lin", &["Authorization", "X-Org_Id"]).check(), Ok(()));
        let refused = |spec: Spec| spec.check().unwrap_err();
        assert_eq!(refused(stdio("my server", None, &[])), "name the server with letters, digits, - and _ (at most 64), not \"my server\"");
        assert_eq!(refused(stdio("Pinned", None, &[])), "pinned is pinned's own server's name");
        assert_eq!(refused(stdio("ctx", Some(" github.com/a/b"), &[])), "a project is its repository, as github.com/owner/repo");
        assert_eq!(refused(Spec { transport: Reach::Stdio { command: " ".into(), args: vec![], env: vec![] }, ..stdio("ctx", None, &[]) }), "ctx runs no command");
        assert_eq!(refused(Spec { transport: Reach::Sse { url: "ftp://x/sse".into(), headers: vec![] }, ..http("ev", &[]) }), "ev's URL must start with http:// or https://");
        let keyed = Spec { transport: Reach::Stdio { command: "srv".into(), args: vec!["--token".into(), "ghp_abcdefghijklmnopqrstuvwxyz".into()], env: vec![] }, ..stdio("gh", None, &[]) };
        assert_eq!(refused(keyed), "gh holds a key in its command, arguments or URL, which lists show: put it in an environment variable or a header");
        let in_url = Spec { transport: Reach::Http { url: "https://x.example.com/mcp?k=lin_api_abcdefghijklmnopqrst".into(), headers: vec![] }, ..http("x", &[]) };
        assert!(refused(in_url).contains("holds a key"));
        assert_eq!(refused(stdio("ctx", None, &["1KEY"])), "ctx: \"1KEY\" is not a valid environment variable name");
        assert_eq!(refused(http("lin", &["Bad Header"])), "lin: \"Bad Header\" is not a valid header name");
        assert_eq!(refused(http("lin", &["X-A", "x-a"])), "lin names x-a twice");
    }

    #[test]
    fn values_cover_each_secret_name_and_nothing_else() {
        let spec = stdio("ctx", None, &["API_KEY", "REGION"]);
        assert_eq!(spec.check_values(&values(&[("API_KEY", "k"), ("REGION", "eu")], &[]), &[]), Ok(()));
        assert_eq!(spec.check_values(&values(&[("API_KEY", "k")], &[]), &[]), Err("ctx needs a value for REGION, which the user types in the app".into()));
        assert_eq!(spec.check_values(&values(&[("API_KEY", " ")], &[]), &["API_KEY".into(), "REGION".into()]), Ok(()), "a blank value keeps the stored one");
        assert_eq!(spec.check_values(&values(&[("OTHER", "x")], &[]), &["API_KEY".into(), "REGION".into()]), Err("ctx has no OTHER to set".into()));
        assert_eq!(spec.check_values(&values(&[], &[("Authorization", "x")]), &["API_KEY".into(), "REGION".into()]), Err("ctx has no Authorization to set".into()));
    }

    fn load(spec: Spec, values: Values) -> Load {
        Load { server: spec, values }
    }

    #[test]
    fn a_session_loads_every_projects_servers_and_its_own_projects_and_leaves_the_accounts_own_names() {
        let loads = [
            load(stdio("ctx", None, &[]), Values::default()),
            load(stdio("db", None, &[]), Values::default()),
            load(stdio("db", Some("github.com/acme/site"), &[]), Values::default()),
            load(stdio("docs", Some("github.com/acme/other"), &[]), Values::default()),
            load(http("linear", &[]), Values::default()),
        ];
        let names = |picked: Vec<&Load>| picked.iter().map(|l| (l.server.name.clone(), l.server.project.clone())).collect::<Vec<_>>();
        assert_eq!(
            names(for_session(&loads, Some("github.com/acme/site"), &["linear".into()])),
            [("ctx".to_owned(), None), ("db".to_owned(), Some("github.com/acme/site".to_owned()))],
            "the project's db replaces the account-wide one, another project's docs stays out, and the account's own linear wins"
        );
        assert_eq!(names(for_session(&loads, None, &[])), [("ctx".to_owned(), None), ("db".to_owned(), None), ("linear".to_owned(), None)]);
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(path).unwrap().permissions()) & 0o777
    }

    #[test]
    fn a_sessions_config_is_its_users_alone_holds_the_secrets_and_goes_with_the_session() {
        let loads = [
            load(stdio("ctx", None, &["API_KEY"]), values(&[("API_KEY", "ctx-secret-1")], &[])),
            load(http("lin", &["Authorization"]), values(&[], &[("Authorization", "Bearer lin-secret-2")])),
            load(Spec { transport: Reach::Sse { url: "http://localhost:8080/sse".into(), headers: vec![] }, ..http("ev", &[]) }, Values::default()),
        ];
        assert!(SessionFiles::write(&[]).unwrap().is_none(), "no servers, no file");
        let files = SessionFiles::write(&loads.iter().collect::<Vec<_>>()).unwrap().unwrap();
        let dir = files.dir().to_owned();
        #[cfg(unix)]
        assert_eq!((mode(&dir), mode(&files.file)), (0o700, 0o600));
        assert_eq!(
            read_json(&files.file).unwrap(),
            json!({ "mcpServers": {
                "ctx": { "type": "stdio", "command": "npx", "args": ["-y", "@acme/ctx"], "env": { "API_KEY": "ctx-secret-1" } },
                "lin": { "type": "http", "url": "https://lin.example.com/mcp", "headers": { "Authorization": "Bearer lin-secret-2" } },
                "ev": { "type": "sse", "url": "http://localhost:8080/sse", "headers": {} },
            } })
        );
        let file = files.file.display().to_string();
        let flags = files.codex_flags(Path::new("/opt/pinned/bin/pinned"));
        assert_eq!(
            flags,
            [
                "-c".to_owned(),
                "mcp_servers.ctx.command=\"/opt/pinned/bin/pinned\"".to_owned(),
                "-c".to_owned(),
                format!("mcp_servers.ctx.args=[\"mcp-run\",\"{file}\",\"ctx\"]"),
                "-c".to_owned(),
                "mcp_servers.lin.url=\"https://lin.example.com/mcp\"".to_owned(),
                "-c".to_owned(),
                format!("mcp_servers.lin.http_headers_helper=\"'/opt/pinned/bin/pinned' 'mcp-headers' '{file}' 'lin'\""),
            ]
        );
        assert!(!flags.concat().contains("secret"), "no secret in Codex's arguments: {flags:?}");
        assert_eq!(headers(&[file.clone(), "lin".into()]), Ok(r#"{"Authorization":"Bearer lin-secret-2"}"#.to_owned()));
        assert_eq!(headers(&[file.clone(), "ctx".into()]), Ok("{}".to_owned()));
        assert_eq!(headers(&[file.clone(), "nope".into()]), Err(format!("{file} has no server nope")));
        assert_eq!(run(&[file.clone()]), Err("takes a session's MCP config file and a server name".into()));
        drop(files);
        assert!(!dir.exists(), "the session's end removes its config");
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_left_by_a_pinned_that_stopped_is_swept_and_a_running_ones_stays() {
        let stale = std::env::temp_dir().join(format!("{DIR_PREFIX}999999-test"));
        let live = std::env::temp_dir().join(format!("{DIR_PREFIX}{}-test", std::process::id()));
        for dir in [&stale, &live] {
            std::fs::create_dir_all(dir).unwrap();
        }
        sweep_stale();
        assert!(!stale.exists() && live.exists());
        std::fs::remove_dir_all(&live).unwrap();
    }

    #[test]
    fn import_reads_commands_args_urls_and_secret_names_never_values() {
        let root = temp("import");
        let home = root.join("accounts/work");
        let site = root.join("site");
        let mut config = json!({
            "mcpServers": {
                "linear": { "type": "http", "url": "https://mcp.linear.app/mcp", "headers": { "Authorization": "Bearer lin_api_secret" } },
                "files": { "command": "files-server", "args": ["--root", "/data"], "env": { "API_KEY": "hunter2" } },
            },
        });
        config["projects"][site.display().to_string()] = json!({ "mcpServers": { "db": { "type": "stdio", "command": "npx", "args": ["-y", "@acme/db"], "env": { "DB_PASSWORD": "hunter3" } } } });
        write(&home.join(".claude.json"), config);
        write(&site.join(".mcp.json"), json!({ "mcpServers": { "events": { "type": "sse", "url": "http://localhost:8080/sse" } } }));
        let found = serde_json::to_value(importable(&home, Some(&site))).unwrap();
        assert_eq!(
            found,
            json!([
                { "scope": "user", "server": { "name": "files", "transport": { "type": "stdio", "command": "files-server", "args": ["--root", "/data"], "env": ["API_KEY"] } } },
                { "scope": "user", "server": { "name": "linear", "transport": { "type": "http", "url": "https://mcp.linear.app/mcp", "headers": ["Authorization"] } } },
                { "scope": "local", "server": { "name": "db", "transport": { "type": "stdio", "command": "npx", "args": ["-y", "@acme/db"], "env": ["DB_PASSWORD"] } } },
                { "scope": "project", "server": { "name": "events", "transport": { "type": "sse", "url": "http://localhost:8080/sse" } } },
            ])
        );
        for secret in ["lin_api_secret", "hunter2", "hunter3"] {
            assert!(!found.to_string().contains(secret), "{secret} in {found}");
        }
    }
}
