//! GitHub, whose token lets any machine of the account clone its private repositories.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The real GitHub, the default base of a `Client`.
pub const API: &str = "https://api.github.com";
/// Where a signed-in user creates the token pinned asks for, with the `repo` scope picked.
pub const TOKEN_PAGE: &str = "https://github.com/settings/tokens/new?scopes=repo&description=pinned";
/// Most repositories `repos` lists, newest push first.
const REPOS_LIMIT: usize = 500;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Account {
    pub login: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Repo {
    /// `owner/name`.
    pub full_name: String,
    pub clone_url: String,
    pub private: bool,
    #[serde(default)]
    pub description: String,
}

/// Why a GitHub call failed; `Display` is the text `account` and `repos` have always returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// GitHub answered 401, or 403 for a reason other than a rate limit: the token is refused or may not do this.
    Unauthorized(String),
    /// The outcome is unknown: the connection broke once the request was out, or the answer did not read. GitHub may
    /// have done what was asked.
    Failed(String),
    /// GitHub answered `status`, which is not a success, a 401 or a token's 403, so it did nothing. `busy` for a rate
    /// limit (a 403 that says so, or a 429) and for a 5xx.
    Answered { status: u16, busy: bool, text: String },
    /// The request never left: the name did not resolve or the connection was refused.
    Unreachable(String),
    /// No answer in time: GitHub may have done what was asked.
    TimedOut(String),
}

impl Error {
    /// Whether GitHub may have done what was asked: the request went out and no answer said it did not.
    pub fn may_have_reached(&self) -> bool {
        matches!(self, Error::Failed(_) | Error::TimedOut(_))
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Error::Unauthorized(text) | Error::Failed(text) | Error::Answered { text, .. } | Error::Unreachable(text) | Error::TimedOut(text)) = self;
        f.write_str(text)
    }
}

impl std::error::Error for Error {}

impl From<Error> for String {
    fn from(error: Error) -> String {
        error.to_string()
    }
}

/// How long one GitHub call may take.
pub const CALL_TIMEOUT: std::time::Duration = crate::connectors::CALL_TIMEOUT;

/// A GitHub API at `api` that `token` signs in to.
#[derive(Clone)]
pub struct Client {
    pub api: String,
    pub token: String,
    /// How long one call may take; `CALL_TIMEOUT` unless a caller asks for another.
    pub timeout: std::time::Duration,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("api", &self.api).finish_non_exhaustive()
    }
}

impl Client {
    /// The real GitHub.
    pub fn new(token: &str) -> Client {
        Client { api: API.into(), token: token.into(), timeout: CALL_TIMEOUT }
    }

    pub async fn get(&self, path: &str) -> Result<Value, Error> {
        self.call("pinned", reqwest::Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value, Error> {
        self.call("pinned", reqwest::Method::POST, path, Some(body)).await
    }

    /// `get` that names itself `agent` in the `User-Agent` header.
    pub async fn get_as(&self, agent: &str, path: &str) -> Result<Value, Error> {
        self.call(agent, reqwest::Method::GET, path, None).await
    }

    /// `post` that names itself `agent` in the `User-Agent` header.
    pub async fn post_as(&self, agent: &str, path: &str, body: &Value) -> Result<Value, Error> {
        self.call(agent, reqwest::Method::POST, path, Some(body)).await
    }

    async fn call(&self, agent: &str, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<Value, Error> {
        // No redirect is followed: a 307 would resend the token and the body to wherever it points.
        let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().map_err(|e| Error::Unreachable(format!("GitHub unreachable: {e}")))?;
        let mut request = http
            .request(method, format!("{}{path}", self.api))
            .timeout(self.timeout)
            .bearer_auth(&self.token)
            .header("User-Agent", agent)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            request = request.json(body);
        }
        // reqwest's error names the URL, never a header, so the token stays out of it.
        let response = request.send().await.map_err(|e| match () {
            _ if e.is_timeout() => Error::TimedOut(format!("GitHub did not answer in time: {e}")),
            _ if e.is_connect() => Error::Unreachable(format!("GitHub unreachable: {e}")),
            _ => Error::Failed(format!("GitHub unreachable: {e}")),
        })?;
        let status = response.status();
        let remaining = response.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let read = response.json::<Value>().await;
        if status.as_u16() == 401 {
            return Err(Error::Unauthorized("GitHub refused the token".into()));
        }
        if !status.is_success() {
            // An answer that is not a success did nothing, whatever its body holds.
            let message = read.as_ref().ok().and_then(|body| body["message"].as_str()).unwrap_or_default().to_owned();
            return Err(match status.as_u16() {
                403 if rate_limited(remaining.as_deref(), &message) => Error::Answered { status: 403, busy: true, text: format!("GitHub rate limit reached, try again later: {message}") },
                403 => Error::Unauthorized(format!("GitHub answered {status}: {message}")),
                code => Error::Answered { status: code, busy: code == 429 || code >= 500, text: format!("GitHub answered {status}: {message}") },
            });
        }
        read.map_err(|e| match () {
            _ if e.is_timeout() => Error::TimedOut(format!("GitHub did not answer in time: {e}")),
            _ => Error::Failed(format!("GitHub sent a bad response: {e}")),
        })
    }
}

/// Whether a 403 means "too many requests" (a temporary failure) rather than "this token may not": GitHub sends
/// `x-ratelimit-remaining: 0`, or says so in the message.
pub fn rate_limited(remaining_header: Option<&str>, message: &str) -> bool {
    remaining_header.is_some_and(|v| v.trim() == "0") || message.to_lowercase().contains("rate limit")
}

async fn get(token: &str, path: &str) -> Result<Value, String> {
    Ok(Client::new(token).get(path).await?)
}

pub async fn account(token: &str) -> Result<Account, String> {
    let user = get(token, "/user").await?;
    let field = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    Ok(Account { login: field(&user["login"]), name: field(&user["name"]) })
}

/// The repositories the token can reach, most recently pushed first.
pub async fn repos(token: &str) -> Result<Vec<Repo>, String> {
    let mut repos = vec![];
    for page in 1.. {
        let found = get(token, &format!("/user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member&page={page}")).await?;
        let found: Vec<Value> = found.as_array().cloned().unwrap_or_default();
        let last = found.len() < 100;
        repos.extend(found.into_iter().filter_map(|r| {
            Some(Repo {
                full_name: r["full_name"].as_str()?.to_owned(),
                clone_url: r["clone_url"].as_str()?.to_owned(),
                private: r["private"].as_bool().unwrap_or_default(),
                description: r["description"].as_str().unwrap_or_default().to_owned(),
            })
        }));
        if last || repos.len() >= REPOS_LIMIT {
            break;
        }
    }
    repos.truncate(REPOS_LIMIT);
    Ok(repos)
}

/// The git config that sends `token` to github.com over https, as actions/checkout sets it.
pub fn auth_config(token: &str) -> (&'static str, String) {
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
    ("http.https://github.com/.extraheader", format!("AUTHORIZATION: basic {basic}"))
}

/// Whether `repo` lives on GitHub, so the account's token may go with it.
pub fn hosts(repo: &str) -> bool {
    clone_url(repo).contains("github.com")
}

/// `owner/name` as a clone URL; any other text is taken as a URL already.
pub fn clone_url(repo: &str) -> String {
    let repo = repo.trim();
    let shorthand = repo.split('/').count() == 2
        && !repo.contains(':')
        && repo.split('/').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)));
    if shorthand {
        format!("https://github.com/{repo}.git")
    } else {
        repo.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shorthand_names_a_github_repository_and_urls_stay_as_they_are() {
        assert_eq!(clone_url(" acme/app "), "https://github.com/acme/app.git");
        assert_eq!(clone_url("git@github.com:acme/app.git"), "git@github.com:acme/app.git");
        assert_eq!(clone_url("https://gitlab.com/acme/app.git"), "https://gitlab.com/acme/app.git");
        assert_eq!(clone_url("acme/app/extra"), "acme/app/extra");
    }

    #[test]
    fn a_403_is_a_rate_limit_by_its_header_or_its_message() {
        assert!(rate_limited(Some("0"), "Forbidden"));
        assert!(rate_limited(Some(" 0 "), ""));
        assert!(rate_limited(None, "API rate limit exceeded for 203.0.113.9."));
        assert!(rate_limited(None, "You have exceeded a secondary Rate Limit"));
        assert!(!rate_limited(Some("4999"), "Resource not accessible by personal access token"));
        assert!(!rate_limited(None, "Forbidden"));
    }

    #[test]
    fn the_token_goes_in_a_basic_header_for_github_only() {
        assert_eq!(
            auth_config("ghp_1"),
            ("http.https://github.com/.extraheader", format!("AUTHORIZATION: basic {}", base64::engine::general_purpose::STANDARD.encode("x-access-token:ghp_1")))
        );
    }
}
