//! Convex, the backend a project may keep its data in. An agent reads a connected deployment through
//! five tools; the hub keeps each deploy key and makes every read.

pub mod access;
pub mod client;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::Path;

/// Where the user creates a deploy key for a deployment.
pub const KEY_PAGE: &str = "https://dashboard.convex.dev/deployment/settings";

/// A deployment's kind, as its deploy key names it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "ConvexKind")]
pub enum Kind {
    Dev,
    Prod,
    Preview,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Dev => "dev",
            Kind::Prod => "prod",
            Kind::Preview => "preview",
        }
    }
}

/// One connected deployment, as Settings shows it. Never holds the key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[schemars(rename = "ConvexConnection")]
pub struct Connection {
    /// The deployment's name, `quick-lemur-789`. Also the connection's id.
    pub id: String,
    pub kind: Kind,
    /// The deployment's address, which Convex resolved from the key. It may be a custom domain.
    pub url: String,
    /// The Convex team's slug.
    #[serde(default)]
    pub team: String,
    /// The Convex project's slug, the name the card shows.
    #[serde(default)]
    pub project: String,
    /// The key's last characters, as `…a1b2`.
    pub key_hint: String,
    /// The pinned projects whose sessions read it, as repo keys (`github.com/acme/site`). A repo key
    /// sits in at most one connection.
    #[serde(default)]
    pub repos: Vec<String>,
}

/// A pasted deploy key. No Debug, no Serialize, no Clone: nothing prints or copies it by accident.
pub struct DeployKey {
    text: String,
    deployment: String,
    kind: Kind,
}

const NOT_A_KEY: &str = "That is not a Convex deploy key. Create one under Deploy keys in the deployment's settings: https://dashboard.convex.dev/deployment/settings";
const NO_DEPLOYMENT: &str = "names no deployment. Create a deploy key in the deployment's settings instead: https://dashboard.convex.dev/deployment/settings";

impl DeployKey {
    /// `dev:<name>|<t>` Dev. `prod:<name>|<t>` Prod. `preview:<name>|<t>` Preview. `<name>|<t>` Prod.
    /// Refuses `preview:<team>:<project>|…` and `project:…`: they name no deployment.
    pub fn parse(text: &str) -> Result<DeployKey, String> {
        let text = text.trim();
        let Some((prefix, secret)) = text.split_once('|') else { return Err(NOT_A_KEY.into()) };
        let (kind, name) = match prefix.split_once(':') {
            Some(("dev", name)) => (Kind::Dev, name),
            Some(("prod", name)) => (Kind::Prod, name),
            Some(("preview", name)) if name.contains(':') => return Err(format!("That is a preview deploy key for a whole project, which {NO_DEPLOYMENT}")),
            Some(("preview", name)) => (Kind::Preview, name),
            Some(("project", _)) => return Err(format!("That is a project key, which {NO_DEPLOYMENT}")),
            Some(_) => return Err(NOT_A_KEY.into()),
            None => (Kind::Prod, prefix),
        };
        let named = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !named || secret.is_empty() || secret.chars().any(char::is_whitespace) {
            return Err(NOT_A_KEY.into());
        }
        Ok(DeployKey { text: text.to_owned(), deployment: name.to_owned(), kind })
    }

    pub fn deployment(&self) -> &str {
        &self.deployment
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn hint(&self) -> String {
        crate::upstash::hint(&self.text)
    }

    fn text(&self) -> &str {
        &self.text
    }

    /// Keeps the key in `storage` under `key_name`.
    pub fn keep(&self, storage: &dyn crate::connectors::Storage) -> Result<(), String> {
        storage.write_key(&key_name(&self.deployment), &self.text)
    }

    /// The key `keep` kept for `deployment`.
    pub fn kept(storage: &dyn crate::connectors::Storage, deployment: &str) -> Result<DeployKey, String> {
        let gone = || format!("pinned lost the deploy key of `{deployment}`. Paste it again on the Convex card in Settings, Connectors.");
        DeployKey::parse(&storage.key(&key_name(deployment)).ok_or_else(gone)?).map_err(|_| gone())
    }

    /// `text` with this key, and the secret after its `|`, replaced by `[hidden]`. Every error text
    /// passes through it.
    pub fn scrub(&self, text: &str) -> String {
        let secret = self.text.split_once('|').map_or("", |(_, secret)| secret);
        let text = text.replace(&self.text, "[hidden]");
        if secret.len() < 8 {
            return text;
        }
        text.replace(secret, "[hidden]")
    }
}

/// Where the hub keeps the deploy key of `deployment`. It has no `:`, so it is a file name on Windows too.
pub fn key_name(deployment: &str) -> String {
    format!("convex-key-{deployment}")
}

/// What a session asks. Tool `convex_<op>` sends `{"op":"<op>",…}`. `Offer` has no tool.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
#[schemars(rename = "ConvexAsk")]
pub enum Ask {
    /// What this tab may read, asked by `pinned mcp` at tools/list.
    Offer,
    /// With no table: the table names. With a table: its fields and indexes.
    Tables {
        #[serde(default)]
        table: Option<String>,
    },
    Data {
        table: String,
        #[serde(default)]
        limit: Option<u32>,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        oldest_first: bool,
    },
    /// The body of `async (ctx) => { … }`, run as a read-only query.
    Query { code: String },
    /// A deployed query by path, `invoices:byNumber`.
    Run {
        function: String,
        #[serde(default)]
        args: Value,
    },
    Logs {
        #[serde(default)]
        function: Option<String>,
        #[serde(default)]
        errors: bool,
        #[serde(default)]
        limit: Option<u32>,
    },
}

impl Ask {
    /// The tool that sends it.
    pub fn tool(&self) -> &'static str {
        match self {
            Ask::Offer => "convex_offer",
            Ask::Tables { .. } => TOOLS[0],
            Ask::Data { .. } => TOOLS[1],
            Ask::Query { .. } => TOOLS[2],
            Ask::Run { .. } => TOOLS[3],
            Ask::Logs { .. } => TOOLS[4],
        }
    }
}

/// What the hub tells `pinned mcp` a tab may read. Travels as a reply value.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Offer {
    pub deployment: String,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tables: Option<Vec<String>>,
}

/// The refusal the machine retries at offer time: it reports a new tab just after it starts it.
pub const UNKNOWN_TAB: &str = "the hub does not know this session's tab yet";
pub const SIGNED_OUT: &str = "Convex needs pinned signed in: the hub keeps the deploy keys.";

pub const TOOLS: [&str; 5] = ["convex_tables", "convex_data", "convex_query", "convex_run", "convex_logs"];

/// The ask a call of tool `name` makes, with the op taken from the name.
pub fn ask(name: &str, arguments: &Value) -> Result<Ask, String> {
    if !TOOLS.contains(&name) {
        return Err(format!("no Convex tool {name}"));
    }
    let mut body = match arguments {
        Value::Object(fields) => fields.clone(),
        Value::Null => Map::new(),
        _ => return Err(format!("{name} takes an object of arguments")),
    };
    body.insert("op".into(), name.trim_start_matches("convex_").into());
    serde_json::from_value(Value::Object(body)).map_err(|e| format!("{name}: {e}"))
}

/// The five tools for `offer`. With a table limit, only tables and data.
pub fn tool_definitions(offer: &Offer) -> Vec<Value> {
    let on = format!("Convex deployment {} ({})", offer.deployment, offer.kind.name());
    let mut after = String::new();
    if let Some(tables) = &offer.tables {
        after.push_str(&format!(" Only the tables {}.", tables.join(", ")));
    }
    if offer.kind == Kind::Prod {
        after.push_str(" This is production data. Quote only what the task needs.");
    }
    let tools = [
        json!({
            "name": TOOLS[0],
            "description": format!("Lists the tables of {on}, which holds this project's data. Pass table for one table's fields and indexes. Start here. Read only.{after}"),
            "inputSchema": {
                "type": "object",
                "properties": { "table": { "type": "string", "description": "One table, for its fields and indexes." } },
            },
        }),
        json!({
            "name": TOOLS[1],
            "description": format!("Reads documents of one table of {on}, newest first. limit is 20 by default and at most 100. Pass next_cursor as cursor to read on.{after}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table": { "type": "string" },
                    "limit": { "type": "integer", "description": "20 by default, at most 100." },
                    "cursor": { "type": "string", "description": "The next_cursor of the page before." },
                    "oldest_first": { "type": "boolean" },
                },
                "required": ["table"],
            },
        }),
        json!({
            "name": TOOLS[2],
            "description": format!("Runs a read-only JavaScript query on {on}. code is the body of async (ctx) => {{ … }}, such as: return await ctx.db.query(\"invoices\").filter((q) => q.eq(q.field(\"number\"), 1042)).collect(); It cannot write.{after}"),
            "inputSchema": {
                "type": "object",
                "properties": { "code": { "type": "string", "description": "The body of async (ctx) => { … }." } },
                "required": ["code"],
            },
        }),
        json!({
            "name": TOOLS[3],
            "description": format!("Runs a deployed query function of {on} by path, with args. The project's convex/ folder defines them: export byNumber in convex/invoices.ts is invoices:byNumber. A mutation or an action is refused.{after}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "function": { "type": "string", "description": "The function's path, as invoices:byNumber." },
                    "args": { "type": "object" },
                },
                "required": ["function"],
            },
        }),
        json!({
            "name": TOOLS[4],
            "description": format!("Shows recent function runs of {on} with their log lines and errors, newest last.{after}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "function": { "type": "string", "description": "Only runs of this function, as invoices:byNumber." },
                    "errors": { "type": "boolean", "description": "Only runs that failed." },
                    "limit": { "type": "integer", "description": "20 by default, at most 50." },
                },
            },
        }),
    ];
    let offered = if offer.tables.is_some() { 2 } else { tools.len() };
    tools.into_iter().take(offered).collect()
}

/// `CONVEX_DEPLOYMENT` from `.env.local`, else `.env`, as written: `dev:quick-lemur-789`. None for a
/// local or anonymous deployment, which no deploy key reaches.
pub fn detect(project: &Path) -> Option<String> {
    [".env.local", ".env"].iter().find_map(|name| deployment_in(&std::fs::read_to_string(project.join(name)).ok()?))
}

fn deployment_in(dotenv: &str) -> Option<String> {
    let value = dotenv.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let (name, value) = line.split_once('=')?;
        (name.trim() == "CONVEX_DEPLOYMENT").then_some(value.trim())
    })?;
    let value = match value.chars().next() {
        Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or_default(),
        _ => value.split(" #").next().unwrap_or_default().trim(),
    };
    let local = value.starts_with("local:") || value.starts_with("anonymous:");
    (!value.is_empty() && !local).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Result<(String, Kind, String), String> {
        DeployKey::parse(text).map(|key| (key.deployment().to_owned(), key.kind(), key.hint()))
    }

    #[test]
    fn a_deploy_key_names_its_deployment_and_kind() {
        assert_eq!(parsed("dev:quick-lemur-789|eyJ2MiI6ImExYjIifQa1b2"), Ok(("quick-lemur-789".into(), Kind::Dev, "…a1b2".into())));
        assert_eq!(parsed(" prod:happy-otter-123|tokenWXYZ\n"), Ok(("happy-otter-123".into(), Kind::Prod, "…WXYZ".into())));
        assert_eq!(parsed("preview:calm-fox-456|token9876"), Ok(("calm-fox-456".into(), Kind::Preview, "…9876".into())));
        assert_eq!(parsed("happy-otter-123|legacy0000"), Ok(("happy-otter-123".into(), Kind::Prod, "…0000".into())));
    }

    #[test]
    fn a_key_that_names_no_deployment_is_refused_with_what_to_create() {
        assert_eq!(
            parsed("preview:acme:site|token").unwrap_err(),
            "That is a preview deploy key for a whole project, which names no deployment. Create a deploy key in the deployment's settings instead: https://dashboard.convex.dev/deployment/settings"
        );
        assert_eq!(
            parsed("project:acme:site|token").unwrap_err(),
            "That is a project key, which names no deployment. Create a deploy key in the deployment's settings instead: https://dashboard.convex.dev/deployment/settings"
        );
        let not_a_key = "That is not a Convex deploy key. Create one under Deploy keys in the deployment's settings: https://dashboard.convex.dev/deployment/settings";
        for text in ["", "quick-lemur-789", "dev:quick-lemur-789|", "dev:|token", "staging:quick-lemur-789|token", "dev:../etc|token", "dev:quick lemur|token", "dev:quick-lemur-789|to ken"] {
            assert_eq!(parsed(text).unwrap_err(), not_a_key, "{text:?}");
        }
    }

    #[test]
    fn a_key_is_kept_under_a_name_without_a_colon() {
        assert_eq!(key_name("quick-lemur-789"), "convex-key-quick-lemur-789");
        assert!(!key_name(DeployKey::parse("dev:quick-lemur-789|t").unwrap().deployment()).contains(':'));
    }

    #[test]
    fn scrub_hides_the_key_and_its_secret_wherever_an_error_echoes_them() {
        let key = DeployKey::parse("dev:quick-lemur-789|eyJ2MiI6InNlY3JldCJ9").unwrap();
        assert_eq!(key.scrub("bad key dev:quick-lemur-789|eyJ2MiI6InNlY3JldCJ9 for quick-lemur-789"), "bad key [hidden] for quick-lemur-789");
        assert_eq!(key.scrub("Convex admin key eyJ2MiI6InNlY3JldCJ9 is invalid"), "Convex admin key [hidden] is invalid");
        assert_eq!(key.scrub("nothing to hide"), "nothing to hide");
    }

    #[test]
    fn each_tool_name_parses_back_to_its_ask() {
        let asks = [
            ("convex_tables", json!({}), Ask::Tables { table: None }),
            ("convex_data", json!({ "table": "invoices", "limit": 5 }), Ask::Data { table: "invoices".into(), limit: Some(5), cursor: None, oldest_first: false }),
            ("convex_query", json!({ "code": "return 1;" }), Ask::Query { code: "return 1;".into() }),
            ("convex_run", json!({ "function": "invoices:byNumber", "args": { "number": 1042 } }), Ask::Run { function: "invoices:byNumber".into(), args: json!({ "number": 1042 }) }),
            ("convex_logs", Value::Null, Ask::Logs { function: None, errors: false, limit: None }),
        ];
        assert_eq!(asks.iter().map(|(name, _, _)| *name).collect::<Vec<_>>(), TOOLS);
        for (name, arguments, expected) in asks {
            assert_eq!(ask(name, &arguments), Ok(expected.clone()), "{name}");
            assert_eq!(expected.tool(), name);
        }
        assert_eq!(ask("convex_offer", &json!({})), Err("no Convex tool convex_offer".into()));
        assert_eq!(ask("convex_tables", &json!({ "table": "invoices" })), Ok(Ask::Tables { table: Some("invoices".into()) }));
        assert_eq!(ask("convex_tables", &json!({ "op": "query", "code": "x" })), Ok(Ask::Tables { table: None }), "the name decides the op");
        assert_eq!(ask("convex_data", &json!({})).unwrap_err(), "convex_data: missing field `table`");
    }

    fn named(tools: &[Value]) -> Vec<(&str, &str)> {
        tools.iter().map(|t| (t["name"].as_str().unwrap(), t["description"].as_str().unwrap())).collect()
    }

    #[test]
    fn a_full_offer_lists_five_tools_that_name_the_deployment() {
        let tools = tool_definitions(&Offer { deployment: "quick-lemur-789".into(), kind: Kind::Dev, tables: None });
        assert_eq!(
            named(&tools),
            [
                ("convex_tables", "Lists the tables of Convex deployment quick-lemur-789 (dev), which holds this project's data. Pass table for one table's fields and indexes. Start here. Read only."),
                ("convex_data", "Reads documents of one table of Convex deployment quick-lemur-789 (dev), newest first. limit is 20 by default and at most 100. Pass next_cursor as cursor to read on."),
                ("convex_query", "Runs a read-only JavaScript query on Convex deployment quick-lemur-789 (dev). code is the body of async (ctx) => { … }, such as: return await ctx.db.query(\"invoices\").filter((q) => q.eq(q.field(\"number\"), 1042)).collect(); It cannot write."),
                ("convex_run", "Runs a deployed query function of Convex deployment quick-lemur-789 (dev) by path, with args. The project's convex/ folder defines them: export byNumber in convex/invoices.ts is invoices:byNumber. A mutation or an action is refused."),
                ("convex_logs", "Shows recent function runs of Convex deployment quick-lemur-789 (dev) with their log lines and errors, newest last."),
            ]
        );
        assert_eq!(tools[1]["inputSchema"]["required"], json!(["table"]));
    }

    #[test]
    fn a_limited_prod_offer_lists_tables_and_data_only_and_warns() {
        let tools = tool_definitions(&Offer { deployment: "happy-otter-123".into(), kind: Kind::Prod, tables: Some(vec!["invoices".into(), "customers".into()]) });
        assert_eq!(
            named(&tools),
            [
                ("convex_tables", "Lists the tables of Convex deployment happy-otter-123 (prod), which holds this project's data. Pass table for one table's fields and indexes. Start here. Read only. Only the tables invoices, customers. This is production data. Quote only what the task needs."),
                ("convex_data", "Reads documents of one table of Convex deployment happy-otter-123 (prod), newest first. limit is 20 by default and at most 100. Pass next_cursor as cursor to read on. Only the tables invoices, customers. This is production data. Quote only what the task needs."),
            ]
        );
    }

    #[test]
    fn an_offer_travels_without_a_table_list_when_it_has_none() {
        let offer = Offer { deployment: "quick-lemur-789".into(), kind: Kind::Dev, tables: None };
        assert_eq!(serde_json::to_value(&offer).unwrap(), json!({ "deployment": "quick-lemur-789", "kind": "dev" }));
        assert_eq!(serde_json::from_value::<Offer>(json!({ "deployment": "quick-lemur-789", "kind": "dev" })).unwrap(), offer);
    }

    #[test]
    fn the_deployment_comes_from_the_projects_dotenv_as_written() {
        let cases = [
            ("CONVEX_DEPLOYMENT=dev:quick-lemur-789 # team: acme, project: site\n", Some("dev:quick-lemur-789")),
            ("# Deployment used by `npx convex dev`\nCONVEX_DEPLOYMENT=dev:quick-lemur-789\nCONVEX_URL=https://quick-lemur-789.convex.cloud\n", Some("dev:quick-lemur-789")),
            ("export CONVEX_DEPLOYMENT=\"prod:happy-otter-123\"\n", Some("prod:happy-otter-123")),
            ("CONVEX_DEPLOYMENT='dev:quick-lemur-789' # team: acme\n", Some("dev:quick-lemur-789")),
            ("  CONVEX_DEPLOYMENT = quick-lemur-789\n", Some("quick-lemur-789")),
            ("CONVEX_DEPLOYMENT=local:local-felipe-site\n", None),
            ("CONVEX_DEPLOYMENT=anonymous:anonymous-site\n", None),
            ("CONVEX_DEPLOYMENT=\n", None),
            ("CONVEX_URL=https://quick-lemur-789.convex.cloud\n", None),
        ];
        for (dotenv, expected) in cases {
            assert_eq!(deployment_in(dotenv).as_deref(), expected, "{dotenv:?}");
        }
    }

    #[test]
    fn detect_reads_dotenv_local_before_dotenv() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(dir.path()), None);
        std::fs::write(dir.path().join(".env"), "CONVEX_DEPLOYMENT=prod:happy-otter-123\n").unwrap();
        assert_eq!(detect(dir.path()).as_deref(), Some("prod:happy-otter-123"));
        std::fs::write(dir.path().join(".env.local"), "CONVEX_DEPLOYMENT=dev:quick-lemur-789\n").unwrap();
        assert_eq!(detect(dir.path()).as_deref(), Some("dev:quick-lemur-789"));
        std::fs::write(dir.path().join(".env.local"), "OTHER=1\n").unwrap();
        assert_eq!(detect(dir.path()).as_deref(), Some("prod:happy-otter-123"), "a .env.local without it falls back to .env");
    }
}
