//! The HTTP side of Convex: every request the connector makes, and what each answer becomes for
//! the agent. The routes are the ones Convex's own CLI and dashboard use.

use super::access::{Grant, Scope};
use super::{Ask, Connection, DeployKey};
use base64::Engine;
use futures_util::future::BoxFuture;
use reqwest::Method;
use serde_json::{json, Map, Value};
use std::path::Path;
use std::time::Duration;

pub const CALL_WAIT: Duration = Duration::from_secs(20);
pub const PAGE_DEFAULT: u32 = 20;
pub const PAGE_MAX: u32 = 100;
pub const LOGS_DEFAULT: u32 = 20;
pub const LOGS_MAX: u32 = 50;
pub const CAP_BYTES: usize = 32 * 1024;
pub const CAP_STRING: usize = 2000;
/// Overrides `API`, for tests and local stubs.
pub const API_BASE_ENV: &str = "CONVEX_API_BASE_URL";
/// What `Reqwest` answers when Convex sends nothing within `CALL_WAIT`. The log route waits for
/// new runs, so for it this means there were none.
pub const TIMED_OUT: &str = "Convex did not answer within 20 seconds.";
const API: &str = "https://api.convex.dev";
const TABLES_PAGE: u32 = 100;
const TABLES_MAX: usize = 500;
/// How many table names a `No table …` answer lists.
const NAMED_MAX: usize = 50;

/// One HTTP exchange. The hub holds `Reqwest`. Tests pass `Replay`.
pub trait Http: Send + Sync {
    fn send<'a>(&'a self, request: Request) -> BoxFuture<'a, Result<Response, String>>;
}

/// No Debug: `auth` and a test-function body hold the key.
pub struct Request {
    pub method: Method,
    pub url: String,
    /// The whole `Authorization` header.
    pub auth: Option<String>,
    pub body: Option<Value>,
}

pub struct Response {
    pub status: u16,
    pub body: Value,
}

/// Every request this client can make. Nothing else builds a Convex path.
enum Route<'a> {
    UrlForKey,
    TeamAndProject,
    Tables { cursor: Option<&'a str> },
    Schemas,
    TableData { table: &'a str, oldest_first: bool, cursor: Option<&'a str>, limit: u32 },
    Run { path: &'a str, args: &'a Value },
    Adhoc { code: &'a str },
    Logs,
}

impl Route<'_> {
    /// The request for `key`. `deployment` is the deployment's address; the two key routes ask
    /// Convex's API instead.
    fn request(&self, key: &DeployKey, deployment: &str) -> Request {
        let admin = Some(format!("Convex {}", key.text()));
        let query = |path: &str, args: Value| Request {
            method: Method::POST,
            url: format!("{deployment}/api/query"),
            auth: admin.clone(),
            body: Some(json!({ "path": path, "format": "convex_encoded_json", "args": [args] })),
        };
        let account = |path: &str| Request {
            method: Method::POST,
            url: format!("{}/api/deployment/{path}", api_base()),
            auth: Some(format!("Bearer {}", key.text())),
            body: Some(json!({ "deployKey": key.text() })),
        };
        match *self {
            Route::UrlForKey => account("url_for_key"),
            Route::TeamAndProject => account("team_and_project_for_key"),
            Route::Tables { cursor } => query("_system/cli/tables", json!({ "paginationOpts": { "cursor": cursor, "numItems": TABLES_PAGE } })),
            Route::Schemas => query("_system/frontend/getSchemas", json!({})),
            Route::TableData { table, oldest_first, cursor, limit } => {
                let order = if oldest_first { "asc" } else { "desc" };
                query("_system/cli/tableData", json!({ "table": table, "order": order, "paginationOpts": { "cursor": cursor, "numItems": limit } }))
            }
            Route::Run { path, args } => query(path, if args.is_null() { json!({}) } else { args.clone() }),
            Route::Adhoc { code } => Request {
                method: Method::POST,
                url: format!("{deployment}/api/run_test_function"),
                auth: admin.clone(),
                body: Some(json!({
                    "adminKey": key.text(),
                    "args": {},
                    "bundle": { "path": "testQuery.js", "source": adhoc_source(code) },
                    "format": "convex_encoded_json",
                })),
            },
            Route::Logs => Request { method: Method::GET, url: format!("{deployment}/api/stream_function_logs?cursor=0"), auth: admin.clone(), body: None },
        }
    }
}

fn adhoc_source(code: &str) -> String {
    format!("import {{ query }} from \"convex:/_system/repl/wrappers.js\";\nexport default query({{ handler: async (ctx) => {{ {code} }} }});\n")
}

/// `CONVEX_API_BASE_URL`, else `https://api.convex.dev`.
fn api_base() -> String {
    std::env::var(API_BASE_ENV).ok().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| API.into()).trim_end_matches('/').to_owned()
}

/// Sends with `CALL_WAIT`, the key in the `Authorization` header. Errors name the host only.
pub struct Reqwest;

impl Http for Reqwest {
    fn send<'a>(&'a self, request: Request) -> BoxFuture<'a, Result<Response, String>> {
        Box::pin(async move {
            let host = reqwest::Url::parse(&request.url).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default();
            let failed = |e: reqwest::Error| if e.is_timeout() { TIMED_OUT.to_owned() } else { format!("Convex at {host} is unreachable.") };
            let mut builder = reqwest::Client::new().request(request.method, &request.url).timeout(CALL_WAIT);
            if let Some(auth) = &request.auth {
                builder = builder.header(reqwest::header::AUTHORIZATION, auth);
            }
            if let Some(body) = &request.body {
                builder = builder.json(body);
            }
            let response = builder.send().await.map_err(failed)?;
            let status = response.status().as_u16();
            let text = response.text().await.map_err(failed)?;
            let body = if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
            Ok(Response { status, body })
        })
    }
}

/// Answers from the fixtures in a folder. Matches by method, path, header and body, with the key
/// placeholders `<admin-key>` and `<deploy-key>` standing for the admin key it was given.
pub struct Replay {
    admin: String,
    fixtures: Vec<Value>,
}

impl Replay {
    pub fn new(dir: &Path, admin_key: &str) -> Result<Replay, String> {
        let mut fixtures = vec![];
        for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let text = text.replace("<admin-key>", admin_key).replace("<deploy-key>", admin_key);
                fixtures.push(serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?);
            }
        }
        Ok(Replay { admin: admin_key.to_owned(), fixtures })
    }

    fn answer(&self, request: &Request) -> Option<Response> {
        let path = request.url.split_once("://").and_then(|(_, rest)| rest.find('/').map(|at| &rest[at..])).unwrap_or_default();
        let auth = match request.auth.as_deref() {
            None => "none",
            Some(header) if header == format!("Convex {}", self.admin) => "admin",
            Some(header) if header == format!("Bearer {}", self.admin) => "bearer",
            Some(_) => "wrong-key",
        };
        let body = request.body.clone().map(source_words).unwrap_or(Value::Null);
        let fixture = self.fixtures.iter().find(|f| {
            let r = &f["request"];
            r["method"] == request.method.as_str() && r["path"] == path && r["auth"] == auth && source_words(r["body"].clone()) == body
        })?;
        Some(Response { status: fixture["status"].as_u64().unwrap_or(200) as u16, body: fixture["response"].clone() })
    }
}

/// A test function's source with its whitespace collapsed, so a recorded multi-line query matches
/// the one-line wrapper.
fn source_words(mut body: Value) -> Value {
    if let Some(source) = body.pointer_mut("/bundle/source") {
        *source = Value::String(source.as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" "));
    }
    body
}

impl Http for Replay {
    fn send<'a>(&'a self, request: Request) -> BoxFuture<'a, Result<Response, String>> {
        let answer = self.answer(&request).ok_or_else(|| format!("no Convex fixture for {} {}", request.method, request.url));
        Box::pin(async move { answer })
    }
}

async fn call(http: &dyn Http, key: &DeployKey, deployment: &str, route: Route<'_>) -> Result<Value, String> {
    outcome(key.deployment(), http.send(route.request(key, deployment)).await?)
}

/// The answer's body, or the error the agent reads.
fn outcome(deployment: &str, response: Response) -> Result<Value, String> {
    let body = response.body;
    let said = |body: &Value| body["message"].as_str().or(body["code"].as_str()).unwrap_or_default().trim().to_owned();
    match response.status {
        200..=299 if body["status"] == "error" => Err(format!("Convex: {}", server_error(body["errorMessage"].as_str().unwrap_or_default()))),
        200..=299 => Ok(body),
        401 => Err(format!("Convex refused the deploy key of `{deployment}`. Paste a new key on the Convex card in Settings, Connectors.")),
        400 if body["code"] == "InvalidModules" => Err(format!("Convex could not compile the query: {}", said(&body))),
        status => Err(format!("Convex answered {status}: {}", said(&body))),
    }
}

/// `message` without the `[Request ID: …] Server Error` line Convex puts first.
fn server_error(message: &str) -> String {
    let message = message.trim();
    let message = match message.split_once('\n') {
        Some((first, rest)) if first.starts_with("[Request ID:") => rest,
        _ => message,
    };
    message.trim().to_owned()
}

/// Resolves the address, team and project of a key, then lists the tables to prove the key reads.
pub async fn resolve(http: &dyn Http, key: &DeployKey) -> Result<Connection, String> {
    resolved(http, key).await.map_err(|e| key.scrub(&e))
}

async fn resolved(http: &dyn Http, key: &DeployKey) -> Result<Connection, String> {
    let url = call(http, key, "", Route::UrlForKey).await?;
    let url = url.as_str().filter(|u| u.starts_with("https://") || u.starts_with("http://")).ok_or("Convex sent no address for this deploy key.")?;
    let url = url.trim_end_matches('/').to_owned();
    let slugs = call(http, key, "", Route::TeamAndProject).await?;
    let slug = |field: &str| slugs[field].as_str().unwrap_or_default().to_owned();
    call(http, key, &url, Route::Tables { cursor: None }).await?;
    Ok(Connection { id: key.deployment().to_owned(), kind: key.kind(), url, team: slug("team"), project: slug("project"), key_hint: key.hint(), repos: vec![] })
}

/// Table names for the workflow step's panel, sorted. The owner asks, so no rule applies.
pub async fn table_names(http: &dyn Http, key: &DeployKey, connection: &Connection) -> Result<Vec<String>, String> {
    names(http, key, &connection.url).await.map_err(|e| key.scrub(&e))
}

async fn names(http: &dyn Http, key: &DeployKey, url: &str) -> Result<Vec<String>, String> {
    let mut names = vec![];
    let mut cursor: Option<String> = None;
    loop {
        let page = value(call(http, key, url, Route::Tables { cursor: cursor.as_deref() }).await?);
        names.extend(page["page"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str().map(str::to_owned)));
        cursor = page["continueCursor"].as_str().map(str::to_owned);
        if page["isDone"] != false || cursor.is_none() || names.len() >= TABLES_MAX {
            break;
        }
    }
    names.truncate(TABLES_MAX);
    names.sort();
    Ok(names)
}

fn value(mut body: Value) -> Value {
    body["value"].take()
}

/// A read for an admitted grant, as the JSON text the agent sees.
pub async fn read(http: &dyn Http, key: &DeployKey, grant: &Grant, ask: Ask) -> Result<String, String> {
    let url = grant.connection.url.as_str();
    let answer = match ask {
        Ask::Offer => Ok(json!(grant.offer()).to_string()),
        Ask::Tables { table: None } => tables(http, key, grant).await,
        Ask::Tables { table: Some(table) } => table_shape(http, key, grant, &table).await,
        Ask::Data { table, limit, cursor, oldest_first } => data(http, key, grant, &table, limit, cursor.as_deref(), oldest_first).await,
        Ask::Query { code } => ran(call(http, key, url, Route::Adhoc { code: &code }).await?),
        Ask::Run { function, args } => run(http, key, url, &function, &args).await,
        Ask::Logs { function, errors, limit } => logs(http, key, url, function.as_deref(), errors, limit).await,
    };
    answer.map_err(|e| key.scrub(&e))
}

/// The table names the grant may see.
async fn visible(http: &dyn Http, key: &DeployKey, grant: &Grant) -> Result<Vec<String>, String> {
    let mut names = names(http, key, &grant.connection.url).await?;
    if let Scope::Tables(allowed) = &grant.scope {
        names.retain(|name| allowed.contains(name));
    }
    Ok(names)
}

async fn tables(http: &dyn Http, key: &DeployKey, grant: &Grant) -> Result<String, String> {
    let names = visible(http, key, grant).await?;
    Ok(json!({ "deployment": grant.connection.id, "kind": grant.connection.kind, "tables": names }).to_string())
}

fn no_table(table: &str, names: &[String]) -> String {
    let listed = names.iter().take(NAMED_MAX).cloned().collect::<Vec<_>>().join(", ");
    match names.len() {
        0 => format!("No table {table}. This deployment has no tables."),
        n if n > NAMED_MAX => format!("No table {table}. Tables: {listed}, and {} more.", n - NAMED_MAX),
        _ => format!("No table {table}. Tables: {listed}."),
    }
}

async fn table_shape(http: &dyn Http, key: &DeployKey, grant: &Grant, table: &str) -> Result<String, String> {
    let names = visible(http, key, grant).await?;
    if !names.iter().any(|name| name == table) {
        return Err(no_table(table, &names));
    }
    let schemas = value(call(http, key, &grant.connection.url, Route::Schemas).await?);
    let active: Value = schemas["active"].as_str().and_then(|text| serde_json::from_str(text).ok()).unwrap_or(Value::Null);
    let declared = active["tables"].as_array().into_iter().flatten().find(|t| t["tableName"] == table);
    let Some(declared) = declared.filter(|t| t["documentType"]["type"] == "object") else {
        return Ok(json!({ "table": table, "fields": null, "note": "No schema. Read a few documents to see its shape." }).to_string());
    };
    let fields: Map<String, Value> = declared["documentType"]["value"].as_object().into_iter().flatten().map(|(name, field)| field_entry(name, field)).collect();
    let indexes: Vec<String> = declared["indexes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|index| {
            let on: Vec<&str> = index["fields"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            format!("{}({})", index["indexDescriptor"].as_str().unwrap_or_default(), on.join(", "))
        })
        .collect();
    Ok(json!({ "table": table, "fields": fields, "indexes": indexes }).to_string())
}

/// A field as `name` (or `name?` when optional) and its type, rendered short.
fn field_entry(name: &str, field: &Value) -> (String, Value) {
    let name = if field["optional"] == true { format!("{name}?") } else { name.to_owned() };
    (name, Value::String(type_text(&field["fieldType"])))
}

fn type_text(t: &Value) -> String {
    let each = |list: &Value| list.as_array().into_iter().flatten().map(type_text).collect::<Vec<_>>();
    match t["type"].as_str().unwrap_or("any") {
        "literal" => t["value"].to_string(),
        "id" => format!("id({})", t["tableName"].as_str().unwrap_or_default()),
        "union" => each(&t["value"]).join(" | "),
        "array" => match type_text(&t["value"]) {
            inner if inner.contains(" | ") => format!("({inner})[]"),
            inner => format!("{inner}[]"),
        },
        "object" => {
            let fields: Vec<String> = t["value"].as_object().into_iter().flatten().map(|(name, field)| {
                let (name, kind) = field_entry(name, field);
                format!("{name}: {}", kind.as_str().unwrap_or_default())
            }).collect();
            format!("{{ {} }}", fields.join(", "))
        }
        "record" => format!("record({}, {})", type_text(&t["keys"]), type_text(&t["values"]["fieldType"])),
        other => other.to_owned(),
    }
}

async fn data(http: &dyn Http, key: &DeployKey, grant: &Grant, table: &str, limit: Option<u32>, cursor: Option<&str>, oldest_first: bool) -> Result<String, String> {
    let limit = limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
    let page = value(call(http, key, &grant.connection.url, Route::TableData { table, oldest_first, cursor, limit }).await?);
    let documents: Vec<Value> = page["page"].as_array().cloned().unwrap_or_default().into_iter().map(clean).collect();
    if documents.is_empty() && cursor.is_none() {
        let names = visible(http, key, grant).await?;
        if !names.iter().any(|name| name == table) {
            return Err(no_table(table, &names));
        }
    }
    let mut answer = json!({ "table": table });
    let (documents, truncated) = fit(documents);
    answer["documents"] = Value::Array(documents);
    if truncated {
        answer["truncated"] = true.into();
    } else if page["isDone"] == false {
        answer["next_cursor"] = page["continueCursor"].clone();
    }
    Ok(answer.to_string())
}

/// The leading documents that fit under `CAP_BYTES`, and whether any were dropped.
fn fit(mut documents: Vec<Value>) -> (Vec<Value>, bool) {
    let mut size: usize = documents.iter().map(|d| d.to_string().len() + 1).sum();
    let mut truncated = false;
    while size > CAP_BYTES {
        let Some(last) = documents.pop() else { break };
        size -= last.to_string().len() + 1;
        truncated = true;
    }
    (documents, truncated)
}

/// `invoices.js:byNumber` as `invoices:byNumber`.
fn function_path(function: &str) -> String {
    let function = function.trim();
    match function.split_once(':') {
        Some((module, name)) => format!("{}:{name}", module.strip_suffix(".js").unwrap_or(module)),
        None => function.to_owned(),
    }
}

async fn run(http: &dyn Http, key: &DeployKey, url: &str, function: &str, args: &Value) -> Result<String, String> {
    let path = function_path(function);
    if path.is_empty() || path.starts_with("_system/") {
        return Err("convex_run runs this project's own queries, not Convex's system functions under _system/.".into());
    }
    if !(args.is_null() || args.is_object()) {
        return Err("args must be an object of the function's arguments.".into());
    }
    ran(call(http, key, url, Route::Run { path: &path, args }).await?)
}

/// A function's value and log lines, once the value fits under `CAP_BYTES`.
fn ran(mut body: Value) -> Result<String, String> {
    let value = clean(body["value"].take());
    let size = value.to_string().len();
    if size > CAP_BYTES {
        return Err(format!("The answer is {size} bytes, over the {} KB a read may return. Return less: fewer documents, fewer fields or a count.", CAP_BYTES / 1024));
    }
    let logs = body["logLines"].take();
    Ok(json!({ "value": value, "logs": if logs.is_null() { json!([]) } else { logs } }).to_string())
}

async fn logs(http: &dyn Http, key: &DeployKey, url: &str, function: Option<&str>, errors: bool, limit: Option<u32>) -> Result<String, String> {
    const NONE: &str = "No recent function runs.";
    let body = match call(http, key, url, Route::Logs).await {
        Err(e) if e == TIMED_OUT => return Ok(NONE.into()),
        other => other?,
    };
    let function = function.map(function_path);
    let limit = limit.unwrap_or(LOGS_DEFAULT).clamp(1, LOGS_MAX) as usize;
    let runs: Vec<&Value> = body["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["kind"] == "Completion")
        .filter(|e| function.as_deref().is_none_or(|f| e["identifier"].as_str().map(function_path).as_deref() == Some(f)))
        .filter(|e| !errors || !e["error"].is_null())
        .collect();
    if runs.is_empty() {
        return Ok(NONE.into());
    }
    let entries: Vec<Value> = runs[runs.len().saturating_sub(limit)..].iter().map(|e| log_entry(e)).collect();
    Ok(json!({ "entries": entries }).to_string())
}

fn log_entry(e: &Value) -> Value {
    let at = chrono::DateTime::from_timestamp_millis((e["timestamp"].as_f64().unwrap_or_default() * 1000.0) as i64)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default();
    let mut entry = json!({
        "at": at,
        "function": function_path(e["identifier"].as_str().unwrap_or_default()),
        "kind": e["udfType"].as_str().unwrap_or_default().to_lowercase(),
        "ok": e["error"].is_null(),
        "ms": (e["executionTime"].as_f64().unwrap_or_default() * 1000.0).round() as u64,
        "lines": clean(e["logLines"].clone()),
    });
    if let Some(error) = e["error"].as_str() {
        entry["error"] = clean(Value::String(error.trim().to_owned()));
    }
    entry
}

/// A Convex value as the agent reads it: whole floats as integers, `$integer`, `$float` and
/// `$bytes` as plain values, and long strings clipped.
fn clean(value: Value) -> Value {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_992.0 => json!(f as i64),
            _ => Value::Number(n),
        },
        Value::String(text) => Value::String(clip(text)),
        Value::Array(items) => Value::Array(items.into_iter().map(clean).collect()),
        Value::Object(fields) => {
            let encoded = fields.iter().next().filter(|_| fields.len() == 1).and_then(|(name, inner)| {
                let text = inner.as_str()?;
                let bytes = base64::engine::general_purpose::STANDARD.decode(text).ok()?;
                match (name.as_str(), <[u8; 8]>::try_from(bytes.as_slice())) {
                    ("$integer", Ok(eight)) => Some(json!(i64::from_le_bytes(eight))),
                    ("$float", Ok(eight)) => Some(Value::String(f64::from_le_bytes(eight).to_string())),
                    ("$bytes", _) => Some(Value::String(clip(format!("{} bytes, base64 {text}", bytes.len())))),
                    _ => None,
                }
            });
            encoded.unwrap_or_else(|| Value::Object(fields.into_iter().map(|(k, v)| (k, clean(v))).collect()))
        }
        other => other,
    }
}

fn clip(text: String) -> String {
    let count = text.chars().count();
    if count <= CAP_STRING {
        return text;
    }
    let kept: String = text.chars().take(CAP_STRING).collect();
    format!("{kept}… [clipped, {} more characters]", count - CAP_STRING)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convex::Kind;
    use std::collections::BTreeSet;

    const ADMIN: &str = "dev:quick-lemur-789|eyJ2MiI6ImFkbWluIn0secret";

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
    }

    fn fixtures() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/convex")
    }

    fn fixture(name: &str) -> Value {
        let text = std::fs::read_to_string(fixtures().join(name)).unwrap();
        serde_json::from_str(&text.replace("<admin-key>", ADMIN).replace("<deploy-key>", ADMIN)).unwrap()
    }

    fn key() -> DeployKey {
        DeployKey::parse(ADMIN).unwrap()
    }

    fn replay() -> Replay {
        Replay::new(&fixtures(), ADMIN).unwrap()
    }

    fn connection() -> Connection {
        Connection { id: "quick-lemur-789".into(), kind: Kind::Dev, url: "http://127.0.0.1:3210".into(), team: "acme".into(), project: "shop".into(), key_hint: "…cret".into(), repos: vec![] }
    }

    fn everything() -> Grant {
        Grant { connection: connection(), scope: Scope::Everything }
    }

    fn only(tables: &[&str]) -> Grant {
        Grant { connection: connection(), scope: Scope::Tables(tables.iter().map(|t| t.to_string()).collect::<BTreeSet<_>>()) }
    }

    fn read_all(grant: &Grant, ask: Ask) -> Result<String, String> {
        block_on(read(&replay(), &key(), grant, ask))
    }

    /// The request as its fixture records it: method, path, auth class and body.
    fn recorded(request: &Request) -> Value {
        let path = request.url.split_once("://").map(|(_, rest)| &rest[rest.find('/').unwrap()..]).unwrap();
        let auth = match request.auth.as_deref() {
            Some(h) if h == format!("Convex {ADMIN}") => "admin",
            Some(h) if h == format!("Bearer {ADMIN}") => "bearer",
            Some(_) => "wrong-key",
            None => "none",
        };
        json!({ "method": request.method.as_str(), "path": path, "auth": auth, "body": request.body.clone().unwrap_or(Value::Null) })
    }

    #[test]
    fn each_route_builds_the_request_its_fixture_recorded_with_no_key_in_a_url() {
        let cursor = fixture("table_data_page1.json")["response"]["value"]["continueCursor"].as_str().unwrap().to_owned();
        let args = json!({ "number": 1042 });
        let empty = json!({});
        let routes = [
            (Route::UrlForKey, "url_for_key.json"),
            (Route::TeamAndProject, "team_and_project_for_key.json"),
            (Route::Tables { cursor: None }, "tables.json"),
            (Route::Schemas, "schemas.json"),
            (Route::TableData { table: "invoices", oldest_first: false, cursor: None, limit: 2 }, "table_data_page1.json"),
            (Route::TableData { table: "invoices", oldest_first: false, cursor: Some(&cursor), limit: 2 }, "table_data_page2.json"),
            (Route::Run { path: "invoices:byNumber", args: &args }, "run_public_query.json"),
            (Route::Run { path: "invoices:countPending", args: &Value::Null }, "run_internal_query.json"),
            (Route::Run { path: "invoices:seed", args: &empty }, "run_mutation_as_query.json"),
            (Route::Adhoc { code: "throw new Error(\"boom\");" }, "test_query_throws.json"),
            (Route::Logs, "logs.json"),
        ];
        for (route, name) in routes {
            let request = route.request(&key(), "http://127.0.0.1:3210");
            assert_eq!(recorded(&request), fixture(name)["request"], "{name}");
            assert!(!request.url.contains("eyJ2"), "{name}: {}", request.url);
        }
        assert!(Route::UrlForKey.request(&key(), "").url.starts_with("https://api.convex.dev/api/deployment/"));
    }

    #[test]
    fn table_names_come_sorted_and_a_limit_keeps_only_its_tables() {
        assert_eq!(read_all(&everything(), Ask::Tables { table: None }), Ok(r#"{"deployment":"quick-lemur-789","kind":"dev","tables":["customers","invoices"]}"#.into()));
        assert_eq!(read_all(&only(&["invoices"]), Ask::Tables { table: None }), Ok(r#"{"deployment":"quick-lemur-789","kind":"dev","tables":["invoices"]}"#.into()));
        assert_eq!(block_on(table_names(&replay(), &key(), &connection())), Ok(vec!["customers".into(), "invoices".into()]));
    }

    #[test]
    fn one_table_answers_its_fields_and_indexes_rendered_short() {
        assert_eq!(
            read_all(&everything(), Ask::Tables { table: Some("invoices".into()) }),
            Ok(r#"{"table":"invoices","fields":{"customer":"id(customers)","number":"number","paidAt":"number | null","status":"\"pending\" | \"paid\""},"indexes":["by_number(number, _creationTime)"]}"#.into())
        );
        assert_eq!(read_all(&everything(), Ask::Tables { table: Some("customers".into()) }), Ok(r#"{"table":"customers","fields":{"name":"string","plan":"string"},"indexes":[]}"#.into()));
        assert_eq!(read_all(&everything(), Ask::Tables { table: Some("nope".into()) }), Err("No table nope. Tables: customers, invoices.".into()));
        assert_eq!(read_all(&only(&["invoices"]), Ask::Tables { table: Some("nope".into()) }), Err("No table nope. Tables: invoices.".into()), "a limit names only its tables");
    }

    #[test]
    fn a_table_the_schema_does_not_declare_says_to_read_documents() {
        let schemas = fixture("schemas.json");
        let tables = fixture("tables.json");
        let mut active: Value = serde_json::from_str(schemas["response"]["value"]["active"].as_str().unwrap()).unwrap();
        active["tables"].as_array_mut().unwrap().retain(|t| t["tableName"] != "invoices");
        let canned = fake(vec![(tables["response"].clone(), 200), (json!({ "status": "success", "value": { "active": active.to_string() } }), 200)]);
        assert_eq!(
            block_on(read(&canned, &key(), &everything(), Ask::Tables { table: Some("invoices".into()) })),
            Ok(r#"{"table":"invoices","fields":null,"note":"No schema. Read a few documents to see its shape."}"#.into())
        );
        let field = |t: Value| type_text(&t);
        assert_eq!(field(json!({ "type": "array", "value": { "type": "union", "value": [{ "type": "string" }, { "type": "null" }] } })), "(string | null)[]");
        assert_eq!(field(json!({ "type": "object", "value": { "a": { "fieldType": { "type": "boolean" }, "optional": true } } })), "{ a?: boolean }");
        assert_eq!(field(json!({ "type": "record", "keys": { "type": "string" }, "values": { "fieldType": { "type": "int64" } } })), "record(string, int64)");
    }

    #[test]
    fn two_data_pages_follow_the_cursor_with_whole_numbers() {
        let first = read_all(&everything(), Ask::Data { table: "invoices".into(), limit: Some(2), cursor: None, oldest_first: false }).unwrap();
        let cursor = fixture("table_data_page1.json")["response"]["value"]["continueCursor"].as_str().unwrap().to_owned();
        assert_eq!(
            first,
            format!(
                r#"{{"table":"invoices","documents":[{{"_creationTime":1794370889263.0469,"_id":"j570ydwjmcsgh6r0kws1fsekmh8j1sf9","customer":"j97bm6eyhz5vkhxw8bd7fcz70x8j1k4b","number":1043,"paidAt":null,"status":"pending"}},{{"_creationTime":1794370889263.0466,"_id":"j570y2r5f2rfmd6sf3ac7knawd8j021h","customer":"j97bzvs5tt8tv5xgqw8f758ve18j0r8s","number":1042,"paidAt":null,"status":"pending"}}],"next_cursor":"{cursor}"}}"#
            )
        );
        assert_eq!(
            read_all(&everything(), Ask::Data { table: "invoices".into(), limit: Some(2), cursor: Some(cursor), oldest_first: false }),
            Ok(r#"{"table":"invoices","documents":[{"_creationTime":1794370889263.0464,"_id":"j57fx6yewmkg0w2442qc6pjj8x8j1yy7","customer":"j97bzvs5tt8tv5xgqw8f758ve18j0r8s","number":1041,"paidAt":1794000000000,"status":"paid"}]}"#.into())
        );
    }

    #[test]
    fn an_unknown_table_names_the_tables_there_are() {
        assert_eq!(read_all(&everything(), Ask::Data { table: "nope".into(), limit: Some(2), cursor: None, oldest_first: false }), Err("No table nope. Tables: customers, invoices.".into()));
        let many: Vec<String> = (0..128).map(|n| format!("table_{n:03}")).collect();
        let named = no_table("nope", &many);
        assert!(named.starts_with("No table nope. Tables: table_000, table_001,"), "{named}");
        assert!(named.ends_with("table_049, and 78 more."), "{named}");
    }

    #[test]
    fn a_deployed_query_answers_its_value_and_logs_and_a_mutation_is_refused() {
        let run = |function: &str, args: Value| read_all(&everything(), Ask::Run { function: function.into(), args });
        assert_eq!(
            run("invoices.js:byNumber", json!({ "number": 1042 })),
            Ok(r#"{"value":{"_creationTime":1794370889263.0466,"_id":"j570y2r5f2rfmd6sf3ac7knawd8j021h","customer":"j97bzvs5tt8tv5xgqw8f758ve18j0r8s","number":1042,"paidAt":null,"status":"pending"},"logs":[]}"#.into())
        );
        assert_eq!(run("invoices:countPending", Value::Null), Ok(r#"{"value":2,"logs":["[LOG] 'counting pending invoices'"]}"#.into()));
        assert_eq!(run("invoices:seed", json!({})), Err("Convex: Trying to execute invoices.js:seed as Query, but it is defined as Mutation.".into()));
        assert_eq!(run("invoices:byNumber", json!({ "number": "x" })), Err("Convex: ArgumentValidationError: Value does not match validator.\nPath: .number\nValue: \"x\"\nValidator: v.float64()".into()));
        assert_eq!(run("invoices:nope", json!({})), Err("Convex: Could not find public function for 'invoices:nope'.".into()));
        assert_eq!(run("_system/cli/tables", json!({})), Err("convex_run runs this project's own queries, not Convex's system functions under _system/.".into()));
        assert_eq!(run("invoices:byNumber", json!([1042])), Err("args must be an object of the function's arguments.".into()));
    }

    #[test]
    fn an_adhoc_query_answers_reads_and_cannot_write() {
        let query = |code: &str| read_all(&everything(), Ask::Query { code: code.into() });
        assert_eq!(
            query("console.log(\"looking up 1042\");\n  return await ctx.db.query(\"invoices\").filter((q) => q.eq(q.field(\"number\"), 1042)).collect();"),
            Ok(r#"{"value":[{"_creationTime":1794370889263.0466,"_id":"j570y2r5f2rfmd6sf3ac7knawd8j021h","customer":"j97bzvs5tt8tv5xgqw8f758ve18j0r8s","number":1042,"paidAt":null,"status":"pending"}],"logs":["[LOG] 'looking up 1042'"]}"#.into())
        );
        assert_eq!(query("await ctx.db.insert(\"customers\", { name: \"X\", plan: \"free\" }); return 1;"), Err("Convex: Uncaught TypeError: ctx.db.insert is not a function".into()));
        assert_eq!(query("throw new Error(\"boom\");"), Err("Convex: Uncaught Error: boom".into()));
        let syntax = fixture("test_query_syntax_error.json");
        assert_eq!(
            outcome("quick-lemur-789", Response { status: 400, body: syntax["response"].clone() }).err(),
            Some("Convex could not compile the query: Could not analyze the given module:\nFailed to analyze testQuery.js: Uncaught SyntaxError: Unexpected end of input".into())
        );
    }

    #[test]
    fn a_wrong_key_is_refused_by_name() {
        let wrong = DeployKey::parse("dev:quick-lemur-789|eyJ2MiI6Indyb25nIn0other").unwrap();
        assert_eq!(
            block_on(read(&replay(), &wrong, &everything(), Ask::Tables { table: None })),
            Err("Convex refused the deploy key of `quick-lemur-789`. Paste a new key on the Convex card in Settings, Connectors.".into())
        );
    }

    #[test]
    fn logs_answer_completed_runs_newest_last() {
        let logs = |function: Option<&str>, errors: bool, limit: Option<u32>| read_all(&everything(), Ask::Logs { function: function.map(str::to_owned), errors, limit });
        assert_eq!(
            logs(None, false, Some(2)),
            Ok(r#"{"entries":[{"at":"2026-11-11T04:21:29Z","function":"invoices:nope","kind":"query","ok":false,"ms":0,"lines":[],"error":"Could not find public function for 'invoices:nope'."},{"at":"2026-11-11T04:21:29Z","function":"invoices:byNumber","kind":"query","ok":false,"ms":0,"lines":[],"error":"ArgumentValidationError: Value does not match validator.\nPath: .number\nValue: \"x\"\nValidator: v.float64()"}]}"#.into())
        );
        assert_eq!(
            logs(Some("invoices.js:countPending"), false, Some(1)),
            Ok(r#"{"entries":[{"at":"2026-11-11T04:21:29Z","function":"invoices:countPending","kind":"query","ok":false,"ms":0,"lines":[],"error":"Could not find public function for 'invoices:countPending'."}]}"#.into())
        );
        assert_eq!(
            logs(Some("invoices:seed"), false, Some(2)),
            Ok(r#"{"entries":[{"at":"2026-11-11T04:21:29Z","function":"invoices:seed","kind":"mutation","ok":true,"ms":3,"lines":[]},{"at":"2026-11-11T04:21:29Z","function":"invoices:seed","kind":"query","ok":false,"ms":0,"lines":[],"error":"Trying to execute invoices.js:seed as Query, but it is defined as Mutation."}]}"#.into())
        );
        assert_eq!(logs(Some("invoices:byNumber"), true, Some(50)).map(|t| t.matches(r#""ok":false"#).count()), Ok(2), "errors keeps only failed runs");
        assert_eq!(logs(Some("invoices:gone"), false, None), Ok("No recent function runs.".into()));
        let quiet = fake(vec![]);
        assert_eq!(block_on(read(&quiet, &key(), &everything(), Ask::Logs { function: None, errors: false, limit: None })), Ok("No recent function runs.".into()));
    }

    #[test]
    fn a_thousand_log_entries_answer_the_newest_twenty() {
        let entries: Vec<Value> = (0..1000)
            .map(|n| json!({ "kind": "Completion", "udfType": "Query", "identifier": format!("f.js:n{n}"), "logLines": [], "timestamp": 1794370877.0 + n as f64, "error": null, "executionTime": 0.001 }))
            .collect();
        let canned = fake(vec![(json!({ "entries": entries, "newCursor": 1 }), 200)]);
        let text = block_on(read(&canned, &key(), &everything(), Ask::Logs { function: None, errors: false, limit: None })).unwrap();
        let answer: Value = serde_json::from_str(&text).unwrap();
        let functions: Vec<&str> = answer["entries"].as_array().unwrap().iter().map(|e| e["function"].as_str().unwrap()).collect();
        assert_eq!(functions, (980..1000).map(|n| format!("f:n{n}")).collect::<Vec<_>>());
        assert!(text.len() < CAP_BYTES, "{}", text.len());
    }

    #[test]
    fn a_hundred_and_twenty_eight_table_names_fit_under_the_cap() {
        let names: Vec<Value> = (0..128).map(|n| json!({ "name": format!("a_rather_long_table_name_for_area_{n:03}") })).collect();
        let canned = fake(vec![
            (json!({ "status": "success", "value": { "page": names[..100], "isDone": false, "continueCursor": "c1" } }), 200),
            (json!({ "status": "success", "value": { "page": names[100..], "isDone": true, "continueCursor": "end" } }), 200),
        ]);
        let text = block_on(read(&canned, &key(), &everything(), Ask::Tables { table: None })).unwrap();
        let answer: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(answer["tables"].as_array().unwrap().len(), 128);
        assert!(text.len() < CAP_BYTES, "{}", text.len());
    }

    #[test]
    fn a_long_answer_is_cut_and_says_so() {
        let documents: Vec<Value> = (0..100).map(|n| json!({ "n": n as f64, "text": "x".repeat(600) })).collect();
        let canned = fake(vec![(json!({ "status": "success", "value": { "page": documents, "isDone": false, "continueCursor": "c" } }), 200)]);
        let text = block_on(read(&canned, &key(), &everything(), Ask::Data { table: "notes".into(), limit: Some(100), cursor: None, oldest_first: false })).unwrap();
        let answer: Value = serde_json::from_str(&text).unwrap();
        assert_eq!((answer["truncated"].clone(), answer.get("next_cursor").is_none()), (json!(true), true));
        assert_eq!(answer["documents"].as_array().unwrap().len(), 52);
        assert_eq!(answer["documents"][51]["n"], json!(51));
        assert!(text.len() <= CAP_BYTES + 100, "{}", text.len());

        let long = clean(json!({ "text": "y".repeat(CAP_STRING + 5), "big": { "$integer": base64::engine::general_purpose::STANDARD.encode(7i64.to_le_bytes()) }, "raw": { "$bytes": "AAEC" } }));
        assert_eq!(long["text"].as_str().unwrap().len(), CAP_STRING + "… [clipped, 5 more characters]".len());
        assert!(long["text"].as_str().unwrap().ends_with("y… [clipped, 5 more characters]"));
        assert_eq!((long["big"].clone(), long["raw"].clone()), (json!(7), json!("3 bytes, base64 AAEC")));

        let huge = fake(vec![(json!({ "status": "success", "value": documents_of(80) }), 200)]);
        assert_eq!(
            block_on(read(&huge, &key(), &everything(), Ask::Query { code: "return await ctx.db.query(\"notes\").collect();".into() })),
            Err(format!("The answer is {} bytes, over the 32 KB a read may return. Return less: fewer documents, fewer fields or a count.", clean(documents_of(80)).to_string().len()))
        );
    }

    fn documents_of(n: usize) -> Value {
        Value::Array((0..n).map(|i| json!({ "i": i, "text": "z".repeat(500) })).collect())
    }

    #[test]
    fn an_error_that_echoes_the_key_comes_out_scrubbed() {
        let echo = fake(vec![(json!({ "status": "error", "errorMessage": format!("[Request ID: 1] Server Error\nbad admin key {ADMIN}") }), 200)]);
        assert_eq!(block_on(read(&echo, &key(), &everything(), Ask::Tables { table: None })), Err("Convex: bad admin key [hidden]".into()));
        let echo = fake(vec![(json!({ "code": "Nope", "message": format!("key eyJ2MiI6ImFkbWluIn0secret was revoked") }), 403)]);
        assert_eq!(block_on(resolve(&echo, &key())), Err("Convex answered 403: key [hidden] was revoked".into()));
    }

    #[test]
    fn resolve_reads_the_address_and_slugs_then_proves_the_key_reads() {
        let resolved = block_on(resolve(&replay(), &key())).unwrap();
        assert_eq!(resolved, Connection { id: "quick-lemur-789".into(), kind: Kind::Dev, url: "https://quick-lemur-789.convex.cloud".into(), team: "acme".into(), project: "shop".into(), key_hint: "…cret".into(), repos: vec![] });
        let no_url = fake(vec![(json!({ "code": "x" }), 200)]);
        assert_eq!(block_on(resolve(&no_url, &key())), Err("Convex sent no address for this deploy key.".into()));
    }

    #[test]
    fn reqwest_sends_the_key_in_the_header_and_not_in_the_path() {
        use std::io::{Read, Write};
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let heard = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = vec![0u8; 0];
            let mut buf = [0u8; 4096];
            while !String::from_utf8_lossy(&seen).contains("\"numItems\"") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
            }
            let body = r#"{"status":"success","value":{"page":[],"isDone":true,"continueCursor":"end"}}"#;
            write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&seen).into_owned()
        });
        let response = block_on(Reqwest.send(Route::Tables { cursor: None }.request(&key(), &address))).unwrap();
        assert_eq!((response.status, response.body["value"]["isDone"].clone()), (200, json!(true)));
        let heard = heard.join().unwrap();
        let (head, _) = heard.split_once("\r\n\r\n").unwrap();
        let first = head.lines().next().unwrap();
        assert_eq!(first, "POST /api/query HTTP/1.1");
        assert!(head.lines().any(|line| line.eq_ignore_ascii_case(&format!("authorization: Convex {ADMIN}"))), "{head}");
        assert!(!first.contains("eyJ2"));

        let refused = block_on(Reqwest.send(Route::Logs.request(&key(), "http://127.0.0.1:9")));
        assert_eq!(refused.err(), Some("Convex at 127.0.0.1 is unreachable.".into()));
    }

    /// Answers each request with the next canned response, then with a timeout.
    struct Fake {
        answers: Vec<(Value, u16)>,
        sent: std::sync::Mutex<usize>,
    }

    fn fake(answers: Vec<(Value, u16)>) -> Fake {
        Fake { answers, sent: std::sync::Mutex::new(0) }
    }

    impl Http for Fake {
        fn send<'a>(&'a self, _: Request) -> BoxFuture<'a, Result<Response, String>> {
            let mut sent = self.sent.lock().unwrap();
            let answer = self.answers.get(*sent).map(|(body, status)| Response { status: *status, body: body.clone() }).ok_or(TIMED_OUT.to_owned());
            *sent += 1;
            Box::pin(async move { answer })
        }
    }
}
