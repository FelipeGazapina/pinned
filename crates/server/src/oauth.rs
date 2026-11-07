//! The hub's OAuth 2.1 authorization server, which lets an external MCP client such as a hosted
//! chat agent act in an account without ever holding its login.
//!
//! A client discovers the hub through RFC 9728 and RFC 8414 metadata, registers itself (RFC 7591,
//! public clients only), and sends the user's browser to `/oauth/authorize` with a PKCE S256
//! challenge. The hub keeps that request and opens the web app on it. The web app is signed in
//! with the account, so the user approves there, through the same socket every other setting
//! goes through, picking the scopes and the projects the agent gets. The approval answers the
//! client's redirect with a one-use code, which the client trades for short-lived tokens bound to
//! the hub's MCP endpoint (RFC 8707).

use crate::db::{self, AgentRequestRow, Spent};
use crate::http::Server;
use crate::hub::{Account, Hub};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use base64::Engine;
use pinned_core::wire::{AgentProject, AgentRedirect, AgentRequest, Call, ExternalAgents, ProjectView};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use subtle::ConstantTimeEq;
use uuid::Uuid;

/// What an agent may be let do, and how the consent card says it.
pub const SCOPES: [(&str, &str); 2] = [
    ("sessions", "Start coding sessions in the projects below, send them messages and read them"),
    ("orchestrator", "Message the orchestrator of the projects below and read its replies"),
];
const MAX_REDIRECTS: usize = 5;
const MAX_PROJECTS: usize = 20;
const REGISTRATIONS: usize = 20;
const TOKEN_FAILURES: usize = 30;
/// How long the approval waits for a machine to list its projects.
const LIST_WAIT: Duration = Duration::from_secs(20);
/// The path the web app shows a pending authorization on.
pub const CONSENT_PATH: &str = "/connect-agent";

/// The MCP endpoint, which every token is issued for.
pub fn resource(public: &str) -> String {
    format!("{public}/mcp")
}

/// Whether a client's `resource` names this hub's MCP endpoint. Clients differ on the trailing
/// slash and some name the server's origin, so both are this resource.
pub fn resource_matches(public: &str, given: &str) -> bool {
    let given = given.trim_end_matches('/');
    given == resource(public) || given == public
}

pub fn protected_resource_metadata(public: &str) -> Value {
    json!({
        "resource": resource(public),
        "authorization_servers": [public],
        "scopes_supported": SCOPES.map(|(s, _)| s),
        "bearer_methods_supported": ["header"],
        "resource_name": "Pinned",
    })
}

pub fn authorization_server_metadata(public: &str) -> Value {
    json!({
        "issuer": public,
        "authorization_endpoint": format!("{public}/oauth/authorize"),
        "token_endpoint": format!("{public}/oauth/token"),
        "registration_endpoint": format!("{public}/oauth/register"),
        "revocation_endpoint": format!("{public}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": SCOPES.map(|(s, _)| s),
        "authorization_response_iss_parameter_supported": true,
    })
}

/// Where a code may be sent: an https page, a loopback http listener as a native app runs, or a
/// native app's own scheme. Never a page that would run the code as script or a remote plain-http one.
pub fn redirect_allowed(uri: &str) -> Result<(), String> {
    let parsed = url::Url::parse(uri).map_err(|_| format!("{uri:?} is not an absolute URL"))?;
    if parsed.fragment().is_some() {
        return Err("a redirect URI may not have a fragment".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("a redirect URI may not carry credentials".into());
    }
    match parsed.scheme() {
        "https" => parsed.host_str().filter(|h| !h.is_empty()).map(drop).ok_or_else(|| "an https redirect URI needs a host".into()),
        "http" => match parsed.host() {
            Some(url::Host::Domain("localhost")) => Ok(()),
            Some(url::Host::Ipv4(ip)) if ip.is_loopback() => Ok(()),
            Some(url::Host::Ipv6(ip)) if ip.is_loopback() => Ok(()),
            _ => Err("a plain http redirect URI must be a loopback address".into()),
        },
        "javascript" | "data" | "file" | "vbscript" | "about" | "blob" | "ftp" | "ws" | "wss" | "filesystem" => Err(format!("{} is not a redirect scheme", parsed.scheme())),
        // RFC 8252's private-use schemes, as a desktop agent registers.
        _ => Ok(()),
    }
}

/// RFC 7636: a verifier is 43 to 128 unreserved characters.
fn valid_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len()) && verifier.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}

/// An S256 challenge is the 43-character unpadded base64url of a sha256.
fn valid_challenge(challenge: &str) -> bool {
    challenge.len() == 43 && challenge.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    if !valid_verifier(verifier) {
        return false;
    }
    let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    computed.as_bytes().ct_eq(challenge.as_bytes()).into()
}

/// The scopes the client asked for that this hub knows, or every scope when it named none of them.
pub fn requested_scopes(scope: Option<&str>) -> Vec<String> {
    let known: Vec<String> = scope.unwrap_or("").split_whitespace().filter(|s| SCOPES.iter().any(|(k, _)| k == s)).map(str::to_owned).collect();
    let mut known = if known.is_empty() { SCOPES.map(|(s, _)| s.to_owned()).to_vec() } else { known };
    known.dedup();
    known
}

/// The redirect URI with `params` added to its query.
pub fn redirect_with(uri: &str, params: &[(&str, &str)]) -> String {
    let mut url = url::Url::parse(uri).expect("a registered redirect URI parses");
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in params {
            query.append_pair(key, value);
        }
    }
    url.to_string()
}

fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

/// RFC 6749's error body.
fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    no_store((status, Json(json!({ "error": error, "error_description": description }))).into_response())
}

/// A page for the person in the browser, for an authorization the client got wrong. Nothing is
/// redirected to an address that is not registered.
fn browser_error(description: &str) -> Response {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Pinned</title><body style=\"font:15px system-ui;margin:3em\"><h1>This connection can't continue</h1><p>{}</p><p>Start connecting again from the agent.</p>",
        description.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    );
    let mut response = no_store((StatusCode::BAD_REQUEST, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response());
    response.headers_mut().insert("content-security-policy", HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'"));
    response
}

pub async fn resource_metadata(State(server): State<Arc<Server>>) -> Response {
    Json(protected_resource_metadata(&server.hub.public_url)).into_response()
}

pub async fn server_metadata(State(server): State<Arc<Server>>) -> Response {
    Json(authorization_server_metadata(&server.hub.public_url)).into_response()
}

pub async fn register(State(server): State<Arc<Server>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if !server.attempt(&[(format!("agent-register:{}", crate::http::client_address(peer, &headers)), REGISTRATIONS)]) {
        return oauth_error(StatusCode::TOO_MANY_REQUESTS, "invalid_client_metadata", "too many registrations; try again in 15 minutes");
    }
    let uris: Vec<String> = match body.get("redirect_uris").and_then(Value::as_array) {
        Some(uris) => uris.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        None => return oauth_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri", "redirect_uris is required"),
    };
    if uris.is_empty() || uris.len() > MAX_REDIRECTS || uris.iter().any(|u| u.len() > 2000) {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri", "register between 1 and 5 redirect URIs");
    }
    if let Some(why) = uris.iter().find_map(|u| redirect_allowed(u).err()) {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri", &why);
    }
    let grants = body.get("grant_types").and_then(Value::as_array).map(|g| g.iter().filter_map(Value::as_str).collect::<Vec<_>>());
    if grants.as_ref().is_some_and(|g| g.iter().any(|g| !["authorization_code", "refresh_token"].contains(g))) {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", "only authorization_code and refresh_token are supported");
    }
    let name: String = body.get("client_name").and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).take(80).collect();
    let name = if name.trim().is_empty() { "An MCP client".to_owned() } else { name.trim().to_owned() };
    match server.hub.db.register_agent_client(&name, &uris).await {
        Ok(id) => no_store(
            (
                StatusCode::CREATED,
                Json(json!({
                    "client_id": id,
                    "client_name": name,
                    "redirect_uris": uris,
                    "grant_types": ["authorization_code", "refresh_token"],
                    "response_types": ["code"],
                    "token_endpoint_auth_method": "none",
                    "client_id_issued_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                })),
            )
                .into_response(),
        ),
        Err(e) => crate::http::internal_response(e),
    }
}

pub async fn authorize(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> Response {
    let hub = &server.hub;
    let param = |k: &str| query.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let Some(client_id) = param("client_id") else { return browser_error("The agent sent no client_id.") };
    let client = match hub.db.agent_client(client_id).await {
        Ok(Some(client)) => client,
        Ok(None) => return browser_error("The agent is not registered with this hub."),
        Err(e) => return crate::http::internal_response(e),
    };
    let redirect_uri = match param("redirect_uri") {
        Some(uri) if client.redirect_uris.iter().any(|r| r == uri) => uri.to_owned(),
        Some(_) => return browser_error("The agent asked to return to an address it did not register."),
        None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
        None => return browser_error("The agent sent no redirect_uri."),
    };
    // From here on the redirect is the client's own, so errors go back to it.
    let state = param("state");
    let fail = |error: &str, description: &str| {
        let mut params = vec![("error", error), ("error_description", description), ("iss", hub.public_url.as_str())];
        if let Some(state) = state {
            params.push(("state", state));
        }
        no_store(Redirect::to(&redirect_with(&redirect_uri, &params)).into_response())
    };
    if param("response_type") != Some("code") {
        return fail("unsupported_response_type", "only the code response type is supported");
    }
    let Some(challenge) = param("code_challenge").filter(|c| valid_challenge(c)) else {
        return fail("invalid_request", "a PKCE code_challenge is required");
    };
    if param("code_challenge_method") != Some("S256") {
        return fail("invalid_request", "code_challenge_method must be S256");
    }
    let resource = match param("resource") {
        Some(given) if !resource_matches(&hub.public_url, given) => return fail("invalid_target", "this hub only issues tokens for its MCP endpoint"),
        _ => self::resource(&hub.public_url),
    };
    let scopes = requested_scopes(param("scope"));
    match hub.db.create_agent_request(&client.id, &redirect_uri, state, challenge, &scopes, &resource).await {
        Ok(request) => no_store(Redirect::to(&format!("{CONSENT_PATH}?request={request}")).into_response()),
        Err(e) => crate::http::internal_response(e),
    }
}

/// A form body, or a JSON one from a client that sends that instead.
fn form(headers: &HeaderMap, body: &[u8]) -> HashMap<String, String> {
    let json = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("json"));
    if json {
        let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        return value.as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect()).unwrap_or_default();
    }
    url::form_urlencoded::parse(body).into_owned().collect()
}

fn tokens(access: String, refresh: String, scopes: &[String]) -> Response {
    no_store(
        Json(json!({
            "access_token": access,
            "token_type": "Bearer",
            "expires_in": db::AGENT_ACCESS_SECS,
            "refresh_token": refresh,
            "scope": scopes.join(" "),
        }))
        .into_response(),
    )
}

pub async fn token(State(server): State<Arc<Server>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let limit = [(format!("agent-token:{}", crate::http::client_address(peer, &headers)), TOKEN_FAILURES)];
    if !server.attempt(&limit) {
        return oauth_error(StatusCode::TOO_MANY_REQUESTS, "invalid_request", "too many failed token requests; try again in 15 minutes");
    }
    let params = form(&headers, &body);
    let answer = exchange(&server.hub, &params).await;
    if answer.status().is_success() {
        server.succeeded(&limit);
    }
    answer
}

async fn exchange(hub: &Hub, params: &HashMap<String, String>) -> Response {
    let db = &hub.db;
    let param = |k: &str| params.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let Some(client_id) = param("client_id") else { return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client", "client_id is required") };
    if let Some(given) = param("resource") {
        if !resource_matches(&hub.public_url, given) {
            return oauth_error(StatusCode::BAD_REQUEST, "invalid_target", "this hub only issues tokens for its MCP endpoint");
        }
    }
    let grant = match param("grant_type") {
        Some("authorization_code") => {
            let (Some(code), Some(verifier)) = (param("code"), param("code_verifier")) else {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "code and code_verifier are required");
            };
            let spent = match db.take_agent_code(code).await {
                Ok(spent) => spent,
                Err(e) => return crate::http::internal_response(e),
            };
            let code = match spent {
                Spent::Fresh(code) => code,
                // RFC 6749 4.1.2: a code used twice ends what it granted, since one of the uses was not the client's.
                Spent::Replayed(grant) => {
                    let _ = db.revoke_agent_grant(grant, None).await;
                    eprintln!("pinned-server: an authorization code was reused; revoked agent grant {grant}");
                    return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "the code was already used or expired");
                }
                Spent::Unknown => return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "unknown code"),
            };
            if code.client != client_id {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "the code was issued to another client");
            }
            if param("redirect_uri").is_some_and(|uri| uri != code.redirect_uri) {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "redirect_uri does not match the authorization request");
            }
            if !pkce_matches(verifier, &code.code_challenge) {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "the code_verifier does not match the code_challenge");
            }
            code.grant
        }
        Some("refresh_token") => {
            let Some(refresh) = param("refresh_token") else { return oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "refresh_token is required") };
            match db.take_agent_refresh(refresh).await {
                Ok(Spent::Fresh((grant, client))) if client == client_id => grant,
                Ok(Spent::Fresh(_)) => return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "the refresh token was issued to another client"),
                // A rotated refresh token came back: someone holds a copy, so the grant ends (OAuth 2.1 4.3.1).
                Ok(Spent::Replayed(grant)) => {
                    let _ = db.revoke_agent_grant(grant, None).await;
                    eprintln!("pinned-server: a rotated refresh token was reused; revoked agent grant {grant}");
                    return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "the refresh token was already used");
                }
                Ok(Spent::Unknown) => return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "unknown or expired refresh token"),
                Err(e) => return crate::http::internal_response(e),
            }
        }
        _ => return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type", "use authorization_code or refresh_token"),
    };
    let scopes = match db.agent_grant_scopes(grant).await {
        Ok(Some(scopes)) => scopes,
        Ok(None) => return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "the access was revoked"),
        Err(e) => return crate::http::internal_response(e),
    };
    match db.issue_agent_tokens(grant).await {
        Ok(Some((access, refresh))) => tokens(access, refresh, &scopes),
        Ok(None) => oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "the access was revoked"),
        Err(e) => crate::http::internal_response(e),
    }
}

/// RFC 7009. Answers 200 for an unknown token too, so it tells nobody which tokens exist.
pub async fn revoke(State(server): State<Arc<Server>>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let params = form(&headers, &body);
    if let Some(token) = params.get("token").filter(|t| !t.is_empty()) {
        if let Err(e) = server.hub.db.revoke_agent_token(token).await {
            return crate::http::internal_response(e);
        }
    }
    no_store(StatusCode::OK.into_response())
}

/// The account's agents, for the Connectors page.
pub async fn agents(hub: &Hub, account: &Account) -> Result<ExternalAgents, String> {
    Ok(ExternalAgents { endpoint: resource(&hub.public_url), agents: hub.db.agent_grants(account.id).await? })
}

fn expired() -> String {
    "This request expired or was already answered. Start connecting again from the agent.".into()
}

pub async fn request(hub: &Hub, id: &str) -> Result<AgentRequest, String> {
    let row = hub.db.agent_request(id).await?.ok_or_else(expired)?;
    Ok(AgentRequest { request: id.to_owned(), client: row.client_name, redirect_uri: row.redirect_uri, scopes: row.scopes, expires_at: row.expires_at })
}

fn answered(hub: &Hub, row: &AgentRequestRow, params: &[(&str, &str)]) -> AgentRedirect {
    let mut all: Vec<(&str, &str)> = params.to_vec();
    all.push(("iss", &hub.public_url));
    if let Some(state) = &row.state {
        all.push(("state", state));
    }
    AgentRedirect { redirect: redirect_with(&row.redirect_uri, &all) }
}

/// Checks each project is one the account's machine lists, and names it as the machine does.
async fn checked_projects(hub: &Hub, account: &Account, projects: Vec<AgentProject>) -> Result<Vec<AgentProject>, String> {
    if projects.is_empty() {
        return Err("Pick at least one project the agent may work in.".into());
    }
    if projects.len() > MAX_PROJECTS {
        return Err(format!("An agent may get at most {MAX_PROJECTS} projects."));
    }
    let mut listed: HashMap<String, Vec<ProjectView>> = HashMap::new();
    let mut checked = vec![];
    for project in projects {
        let machine = Uuid::parse_str(&project.machine).map_err(|_| format!("this account has no machine {}", project.machine))?;
        let name = hub.db.machine_name(account.id, machine).await?.ok_or_else(|| format!("this account has no machine {}", project.machine))?;
        if account.dedicated(&project.machine) {
            return Err(format!("{name} is an Upstash box, which an agent can't be given"));
        }
        if !listed.contains_key(&project.machine) {
            let found = hub.call_machine(account, &project.machine, Call::ListProjects, LIST_WAIT).await.map_err(|e| format!("{name} could not list its projects: {}", String::from(e)))?;
            listed.insert(project.machine.clone(), serde_json::from_value(found).map_err(|e| e.to_string())?);
        }
        let view = listed[&project.machine].iter().find(|p| p.path == project.path).ok_or_else(|| format!("{} is not a project on {name}", project.path))?;
        let entry = AgentProject { machine: project.machine, path: view.path.clone(), name: view.name.clone() };
        if !checked.contains(&entry) {
            checked.push(entry);
        }
    }
    Ok(checked)
}

pub async fn approve(hub: &Hub, account: &Account, id: &str, scopes: Vec<String>, projects: Vec<AgentProject>) -> Result<AgentRedirect, String> {
    hub.db.agent_request(id).await?.ok_or_else(expired)?;
    let mut granted: Vec<String> = SCOPES.iter().map(|(s, _)| s.to_string()).filter(|s| scopes.contains(s)).collect();
    granted.dedup();
    if granted.is_empty() || scopes.iter().any(|s| !SCOPES.iter().any(|(k, _)| k == s)) {
        return Err("Pick what the agent may do.".into());
    }
    let projects = checked_projects(hub, account, projects).await?;
    // Spent only now, so a project the machine doesn't list leaves the request to try again.
    let row = hub.db.take_agent_request(id).await?.ok_or_else(expired)?;
    let code = hub.db.create_agent_grant(account.id, &row, &granted, &projects).await?;
    eprintln!("pinned-server: account {} let agent client {} in", account.id, row.client);
    Ok(answered(hub, &row, &[("code", &code)]))
}

pub async fn deny(hub: &Hub, id: &str) -> Result<AgentRedirect, String> {
    let row = hub.db.take_agent_request(id).await?.ok_or_else(expired)?;
    Ok(answered(hub, &row, &[("error", "access_denied"), ("error_description", "the user refused")]))
}

pub async fn revoke_grant(hub: &Hub, account: &Account, id: &str) -> Result<(), String> {
    let grant = Uuid::parse_str(id).map_err(|_| format!("this account has no agent {id}"))?;
    if !hub.db.revoke_agent_grant(grant, Some(account.id)).await? {
        return Err(format!("this account has no agent {id}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_takes_only_the_verifier_that_hashes_to_the_challenge() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(valid_challenge(challenge));
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXK", challenge));
        assert!(!pkce_matches("short", challenge), "a verifier under 43 characters");
        assert!(!pkce_matches(challenge, challenge), "the challenge is not its own verifier");
        assert!(!valid_challenge("plain-challenge"));
    }

    #[test]
    fn only_https_loopback_http_and_app_schemes_may_receive_a_code() {
        for ok in ["https://grok.example/oauth/callback", "http://127.0.0.1:33418/callback", "http://localhost:8080/cb", "http://[::1]:9/cb", "cursor://anysphere.cursor-mcp/oauth/callback"] {
            assert_eq!(redirect_allowed(ok), Ok(()), "{ok}");
        }
        for bad in ["http://evil.example/cb", "javascript:alert(1)", "data:text/html,hi", "https://grok.example/cb#frag", "https://user:pw@grok.example/cb", "/relative", "file:///etc/passwd"] {
            assert!(redirect_allowed(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_resource_is_the_mcp_endpoint_with_or_without_a_slash() {
        let public = "https://hub.example";
        assert!(resource_matches(public, "https://hub.example/mcp"));
        assert!(resource_matches(public, "https://hub.example/mcp/"));
        assert!(resource_matches(public, "https://hub.example"));
        assert!(!resource_matches(public, "https://other.example/mcp"));
        assert!(!resource_matches(public, "https://hub.example/mcp/other"));
    }

    #[test]
    fn a_client_that_names_no_known_scope_is_offered_every_scope() {
        assert_eq!(requested_scopes(None), vec!["sessions", "orchestrator"]);
        assert_eq!(requested_scopes(Some("openid offline_access")), vec!["sessions", "orchestrator"]);
        assert_eq!(requested_scopes(Some("orchestrator")), vec!["orchestrator"]);
    }

    #[test]
    fn the_metadata_points_clients_at_this_hub() {
        let resource = protected_resource_metadata("https://hub.example");
        assert_eq!(resource["resource"], "https://hub.example/mcp");
        assert_eq!(resource["authorization_servers"], json!(["https://hub.example"]));
        let server = authorization_server_metadata("https://hub.example");
        assert_eq!(server["code_challenge_methods_supported"], json!(["S256"]));
        assert_eq!(server["registration_endpoint"], "https://hub.example/oauth/register");
        assert_eq!(server["token_endpoint_auth_methods_supported"], json!(["none"]));
    }

    #[test]
    fn a_redirect_keeps_the_clients_own_query() {
        assert_eq!(redirect_with("https://a.example/cb?x=1", &[("code", "c d"), ("state", "s")]), "https://a.example/cb?x=1&code=c+d&state=s");
    }
}
