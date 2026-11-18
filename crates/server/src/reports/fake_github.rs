//! An in-memory GitHub repository behind the routes `github::Github` calls, for tests and the dev
//! stack. It takes tokens that start `fake-` and nothing else, and logs every request except the
//! `Authorization` header. Of that header it keeps only a `fake-` token; any other is kept as `NOT_FAKE`.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, MutexGuard};

/// The repository every route serves, whatever `owner/name` the path says.
const HTML_BASE: &str = "https://github.com/acme/app";
/// Seconds since the Unix epoch of 2026-01-01T00:00:00Z, where the fake's clock starts.
const CLOCK_START: i64 = 1_767_225_600;
/// The login of the hub's token, as `GET /user` says it: the author of what the hub posts.
pub const HUB_LOGIN: &str = "pinned-hub";
/// The author of the issues a test puts in the repository unless it names one.
const OTHER_LOGIN: &str = "octo-reporter";
/// The only headers kept of a request besides method, path and body.
const KEPT_HEADERS: [&str; 3] = ["accept", "user-agent", "x-github-api-version"];

/// What `tokens` holds for a request whose token is missing or does not start `fake-`: never the value.
pub const NOT_FAKE: &str = "(not a fake- token)";

/// One request the fake received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// With its query string.
    pub path: String,
    pub body: String,
}

#[derive(Debug, Clone)]
struct StoredComment {
    id: u64,
    login: String,
    body: String,
    created_at: String,
    association: String,
}

#[derive(Debug, Clone)]
struct StoredIssue {
    title: String,
    body: String,
    labels: Vec<String>,
    open: bool,
    state_reason: Option<String>,
    closed_at: Option<String>,
    comments: Vec<StoredComment>,
    timeline: Vec<Value>,
    locked: bool,
    /// The number is a pull request's.
    pull: bool,
    /// Who opened it.
    login: String,
    /// When it was made and last changed, by the fake's clock.
    created_at: String,
    updated_at: String,
}

#[derive(Default)]
struct Repo {
    issues: Vec<StoredIssue>,
    releases: Vec<Value>,
    comment_ids: u64,
    /// Seconds the clock has moved on from `CLOCK_START`; every new thing happens a second later.
    ticks: i64,
    requests: Vec<Request>,
    /// The token of each request, as `tokens` tells it.
    tokens: Vec<String>,
    /// Answers the next request with this instead of serving it: status, message and `x-ratelimit-remaining`.
    fail: Option<(u16, String, Option<String>)>,
    /// Serves the next POST, then holds its answer this long.
    hang: Option<std::time::Duration>,
    /// Waits this long before it creates each issue.
    slow_create: Option<std::time::Duration>,
    last_headers: Vec<(String, String)>,
    /// Seconds since the Unix epoch where the clock starts; `CLOCK_START` unless a test moved it.
    clock_start: Option<i64>,
    /// The `author_association` of the comments the hub posts: `NONE` unless a test says the token is the owner's,
    /// as it is on GitHub.
    hub_association: Option<String>,
    /// Search finds nothing, as when GitHub's index lags or no issue changed since the time asked.
    search_blind: bool,
    /// The next searches that find nothing, as when GitHub's index has not caught up yet.
    blind_searches: u32,
    /// Serves the next POST, then answers it with a success whose body does not read.
    garble: bool,
    /// Serves the next POST, then answers it with this status, as a GitHub that did the work and then failed.
    fail_after_post: Option<u16>,
}

impl Repo {
    fn now(&mut self) -> String {
        self.ticks += 1;
        self.clock()
    }

    /// The clock's time, without moving it on.
    fn clock(&self) -> String {
        iso(self.clock_start.unwrap_or(CLOCK_START) + self.ticks)
    }

    fn comment(&mut self, number: u64, login: &str, association: &str, body: &str, created_at: Option<&str>) -> Option<u64> {
        let created_at = match created_at {
            Some(at) => at.to_owned(),
            None => self.now(),
        };
        self.comment_ids += 1;
        let id = self.comment_ids;
        let issue = self.issues.get_mut(usize::try_from(number).ok()?.checked_sub(1)?)?;
        issue.updated_at = issue.updated_at.clone().max(created_at.clone());
        issue.comments.push(StoredComment { id, login: login.into(), body: body.into(), created_at, association: association.into() });
        Some(id)
    }
}

pub use super::poller::iso;

type Shared = Arc<Mutex<Repo>>;

fn lock(shared: &Shared) -> MutexGuard<'_, Repo> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The running fake and the handle tests use to set it up and look at it.
pub struct FakeGithub {
    /// `http://127.0.0.1:<port>`: the API root.
    pub url: String,
    pub state: FakeState,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for FakeGithub {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// The repository's contents, shared with the server task.
#[derive(Clone)]
pub struct FakeState(Shared);

/// Starts a fake on a free port of 127.0.0.1.
pub async fn start() -> FakeGithub {
    serve("127.0.0.1:0").await.expect("the fake GitHub could not listen")
}

/// Starts a fake on `listen`.
pub async fn serve(listen: &str) -> Result<FakeGithub, String> {
    let shared: Shared = Shared::default();
    let listener = tokio::net::TcpListener::bind(listen).await.map_err(|e| format!("could not listen on {listen}: {e}"))?;
    let url = format!("http://{}", listener.local_addr().map_err(|e| e.to_string())?);
    let app = Router::new().fallback(handle).with_state(shared.clone());
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(FakeGithub { url, state: FakeState(shared), server })
}

impl FakeGithub {
    fn repo(&self) -> MutexGuard<'_, Repo> {
        lock(&self.state.0)
    }

    /// Every request so far, oldest first.
    pub fn requests(&self) -> Vec<Request> {
        self.repo().requests.clone()
    }

    /// The bearer token of every request so far, oldest first: a `fake-` token as sent, anything else as `NOT_FAKE`.
    pub fn tokens(&self) -> Vec<String> {
        self.repo().tokens.clone()
    }

    /// `accept`, `user-agent` and `x-github-api-version` of the latest request, sorted by name.
    pub fn last_headers(&self) -> Vec<(String, String)> {
        self.repo().last_headers.clone()
    }

    pub fn labels(&self, number: u64) -> Vec<String> {
        self.issue(number, |issue| issue.labels.clone())
    }

    pub fn body(&self, number: u64) -> String {
        self.issue(number, |issue| issue.body.clone())
    }

    pub fn comment_bodies(&self, number: u64) -> Vec<String> {
        self.issue(number, |issue| issue.comments.iter().map(|c| c.body.clone()).collect())
    }

    fn issue<T>(&self, number: u64, read: impl FnOnce(&StoredIssue) -> T) -> T {
        let repo = self.repo();
        read(number.checked_sub(1).and_then(|i| repo.issues.get(i as usize)).expect("no such issue in the fake"))
    }

    /// Closes issue `number` as GitHub does: with a reason (`completed`, `not_planned`) and the
    /// commit that did it, if any.
    pub fn close(&self, number: u64, reason: &str, commit: Option<&str>) {
        let mut repo = self.repo();
        let at = repo.now();
        let issue = repo.issues.get_mut(number as usize - 1).expect("no such issue in the fake");
        issue.open = false;
        issue.state_reason = Some(reason.into());
        issue.closed_at = Some(at.clone());
        issue.updated_at = at.clone();
        issue.timeline.push(json!({ "event": "closed", "created_at": at, "commit_id": commit, "state_reason": reason }));
    }

    /// Reopens issue `number` as GitHub does.
    pub fn reopen(&self, number: u64) {
        let mut repo = self.repo();
        let at = repo.now();
        let issue = repo.issues.get_mut(number as usize - 1).expect("no such issue in the fake");
        issue.open = true;
        issue.state_reason = Some("reopened".into());
        issue.closed_at = None;
        issue.updated_at = at.clone();
        issue.timeline.push(json!({ "event": "reopened", "created_at": at }));
    }

    /// Puts `label` on issue `number`, which updates it, as on GitHub.
    pub fn add_label(&self, number: u64, label: &str) {
        let mut repo = self.repo();
        let at = repo.now();
        let issue = repo.issues.get_mut(number as usize - 1).expect("no such issue in the fake");
        issue.labels.push(label.into());
        issue.updated_at = at;
    }

    /// Starts the clock at `seconds` since the Unix epoch instead of 2026-01-01, so what happens in the fake is dated
    /// like what a hub under test does. Moves nothing already dated.
    pub fn set_clock(&self, seconds: i64) {
        self.repo().clock_start = Some(seconds);
    }

    /// From now on the comments the hub posts say they are by `association` (`OWNER`: the hub's token is the owner's).
    pub fn set_hub_association(&self, association: &str) {
        self.repo().hub_association = Some(association.into());
    }

    /// A pull request #`pull` that mentions the issue and was merged at `merged_at`.
    pub fn add_merged_pull(&self, number: u64, pull: u64, merged_at: &str) {
        let mut repo = self.repo();
        let at = repo.now();
        let issue = repo.issues.get_mut(number as usize - 1).expect("no such issue in the fake");
        issue.timeline.push(json!({
            "event": "cross-referenced",
            "created_at": at,
            "source": { "type": "issue", "issue": { "number": pull, "pull_request": { "merged_at": merged_at } } },
        }));
    }

    /// Puts an issue at exactly `number`, as GitHub numbers them: the numbers before it that are free become closed
    /// placeholders, so the next created issue is `number + 1`. Panics when `number` is taken.
    pub fn add_issue_numbered(&self, number: u64, title: &str, body: &str) {
        let mut repo = self.repo();
        let at = repo.clock();
        let blank = |open| StoredIssue { title: String::new(), body: String::new(), labels: vec![], open, state_reason: None, closed_at: None, comments: vec![], timeline: vec![], locked: false, pull: false, login: OTHER_LOGIN.into(), created_at: at.clone(), updated_at: at.clone() };
        assert!(repo.issues.len() < number as usize, "issue {number} is taken in the fake");
        while repo.issues.len() + 1 < number as usize {
            repo.issues.push(blank(false));
        }
        repo.issues.push(StoredIssue { title: title.into(), body: body.into(), ..blank(true) });
    }

    /// Opens the next issue, or with `pull` pull request, as `login`, now; returns its number.
    pub fn add_issue_by(&self, login: &str, title: &str, body: &str, pull: bool) -> u64 {
        let mut repo = self.repo();
        let at = repo.now();
        repo.issues.push(StoredIssue { title: title.into(), body: body.into(), labels: vec![], open: true, state_reason: None, closed_at: None, comments: vec![], timeline: vec![], locked: false, pull, login: login.into(), created_at: at.clone(), updated_at: at });
        repo.issues.len() as u64
    }

    /// The next request is answered `status` with `message`, and `remaining` as `x-ratelimit-remaining`; the ones
    /// after it are served as usual.
    pub fn fail_next(&self, status: u16, message: &str, remaining: Option<&str>) {
        self.repo().fail = Some((status, message.into(), remaining.map(str::to_owned)));
    }

    /// Serves the next POST as usual (the issue or comment it creates exists), then holds the answer for `wait`.
    pub fn hang_next_post(&self, wait: std::time::Duration) {
        self.repo().hang = Some(wait);
    }

    /// Every create of an issue waits `wait` before it is done.
    pub fn slow_creates(&self, wait: std::time::Duration) {
        self.repo().slow_create = Some(wait);
    }

    /// Locks issue `number`: only collaborators may comment.
    pub fn lock(&self, number: u64) {
        self.repo().issues.get_mut(number as usize - 1).expect("no such issue in the fake").locked = true;
    }

    /// Makes `number` a pull request's.
    pub fn make_pull(&self, number: u64) {
        self.repo().issues.get_mut(number as usize - 1).expect("no such issue in the fake").pull = true;
    }

    /// How many issues the repository holds.
    pub fn issue_count(&self) -> usize {
        self.repo().issues.len()
    }

    pub fn add_release(&self, tag: &str, published_at: &str) {
        self.add_release_as(tag, published_at, false, false);
    }

    /// A release that may be a draft or a prerelease.
    pub fn add_release_as(&self, tag: &str, published_at: &str, draft: bool, prerelease: bool) {
        self.repo().releases.push(json!({ "tag_name": tag, "published_at": published_at, "draft": draft, "prerelease": prerelease }));
    }

    /// A comment by `login` with GitHub's `author_association` (`OWNER`, `NONE`, ...), written now; returns its id.
    pub fn add_comment_as(&self, number: u64, login: &str, association: &str, body: &str) -> u64 {
        self.repo().comment(number, login, association, body, None).expect("no such issue in the fake")
    }

    /// A comment by `login` with `association` that says it was written at `created_at`; returns its id.
    pub fn add_comment_full(&self, number: u64, login: &str, association: &str, body: &str, created_at: &str) -> u64 {
        self.repo().comment(number, login, association, body, Some(created_at)).expect("no such issue in the fake")
    }

    /// While `blind`, a search answers with no issues at all.
    pub fn set_search_blind(&self, blind: bool) {
        self.repo().search_blind = blind;
    }

    /// The next search answers with no issues at all, as GitHub's index does before it catches up; the list of
    /// issues still has them.
    pub fn blind_next_search(&self) {
        self.repo().blind_searches += 1;
    }

    /// Serves the next POST as usual (the issue or comment it creates exists), then answers it 201 with a body that
    /// is not JSON.
    pub fn garble_next_post(&self) {
        self.repo().garble = true;
    }

    /// Serves the next POST as usual (the issue or comment it creates exists), then answers it `status`.
    pub fn fail_after_next_post(&self, status: u16) {
        self.repo().fail_after_post = Some(status);
    }

    /// A comment by `login` that says it was written at `created_at`.
    pub fn add_comment_at(&self, number: u64, login: &str, body: &str, created_at: &str) -> u64 {
        self.repo().comment(number, login, "NONE", body, Some(created_at)).expect("no such issue in the fake")
    }
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, axum::Json(body)).into_response()
}

fn not_found() -> Response {
    reply(StatusCode::NOT_FOUND, json!({ "message": "Not Found" }))
}

fn issue_json(number: usize, issue: &StoredIssue) -> Value {
    json!({
        "number": number,
        "title": issue.title,
        "body": issue.body,
        "state": if issue.open { "open" } else { "closed" },
        "state_reason": issue.state_reason,
        "html_url": format!("{HTML_BASE}/issues/{number}"),
        "comments": issue.comments.len(),
        "closed_at": issue.closed_at,
        "locked": issue.locked,
        "pull_request": issue.pull.then(|| json!({ "url": format!("{HTML_BASE}/pull/{number}") })),
        "user": { "login": issue.login },
        "created_at": issue.created_at,
        "updated_at": issue.updated_at,
        "labels": issue.labels.iter().map(|name| json!({ "name": name })).collect::<Vec<_>>(),
    })
}

fn query_of(uri: &Uri) -> Vec<(String, String)> {
    url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).into_owned().collect()
}

async fn handle(State(shared): State<Shared>, method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    let (hang, slow) = {
        let mut repo = lock(&shared);
        let creates = method == Method::POST && uri.path().ends_with("/issues");
        let hang = if method == Method::POST { repo.hang.take() } else { None };
        (hang, repo.slow_create.filter(|_| creates))
    };
    if let Some(wait) = slow {
        tokio::time::sleep(wait).await;
    }
    let (garble, fail_after) = match method == Method::POST {
        true => {
            let mut repo = lock(&shared);
            (std::mem::take(&mut repo.garble), repo.fail_after_post.take())
        }
        false => (false, None),
    };
    let response = serve_now(&shared, method, uri, headers, body);
    let response = match (garble, fail_after) {
        (true, _) => (StatusCode::CREATED, [("content-type", "application/json")], "{\"number\": ").into_response(),
        (false, Some(status)) => reply(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), json!({ "message": "Server Error" })),
        (false, None) => response,
    };
    if let Some(wait) = hang {
        tokio::time::sleep(wait).await;
    }
    response
}

fn serve_now(shared: &Shared, method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    let body_text = String::from_utf8_lossy(&body).into_owned();
    let token = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).filter(|token| token.starts_with("fake-"));
    let authorized = token.is_some();
    let mut repo = lock(shared);
    repo.tokens.push(token.unwrap_or(NOT_FAKE).to_owned());
    repo.requests.push(Request { method: method.to_string(), path: uri.path_and_query().map(|p| p.as_str().to_owned()).unwrap_or_default(), body: body_text.clone() });
    repo.last_headers = KEPT_HEADERS.iter().filter_map(|name| Some((name.to_string(), headers.get(*name)?.to_str().ok()?.to_owned()))).collect();
    if let Some((status, message, remaining)) = repo.fail.take() {
        let mut response = reply(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), json!({ "message": message }));
        if let Some(value) = remaining.and_then(|v| axum::http::HeaderValue::from_str(&v).ok()) {
            response.headers_mut().insert("x-ratelimit-remaining", value);
        }
        return response;
    }
    if !authorized {
        return reply(StatusCode::UNAUTHORIZED, json!({ "message": "Bad credentials" }));
    }
    let query = query_of(&uri);
    let parts: Vec<&str> = uri.path().trim_matches('/').split('/').collect();
    let sent: Value = serde_json::from_str(&body_text).unwrap_or(Value::Null);
    let number_at = |at: usize, repo: &Repo| parts.get(at)?.parse::<usize>().ok().filter(|n| (1..=repo.issues.len()).contains(n));
    match (method.as_str(), parts.as_slice()) {
        ("POST", ["repos", _, _, "issues"]) => {
            let Some(title) = sent["title"].as_str().filter(|t| !t.is_empty()) else {
                return reply(StatusCode::UNPROCESSABLE_ENTITY, json!({ "message": "Validation Failed" }));
            };
            let labels = sent["labels"].as_array().map(|l| l.iter().filter_map(|l| l.as_str().map(str::to_owned)).collect()).unwrap_or_default();
            let at = repo.clock();
            let issue = StoredIssue { title: title.into(), body: sent["body"].as_str().unwrap_or_default().into(), labels, open: true, state_reason: None, closed_at: None, comments: vec![], timeline: vec![], locked: false, pull: false, login: HUB_LOGIN.into(), created_at: at.clone(), updated_at: at };
            repo.issues.push(issue.clone());
            reply(StatusCode::CREATED, issue_json(repo.issues.len(), &issue))
        }
        ("GET", ["repos", _, _, "issues"]) => {
            // GitHub's list: `state` (open unless all or closed), changed at or after `since`, newest made first.
            let get = |key: &str| query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
            let state = get("state").unwrap_or("open");
            let since = get("since").unwrap_or_default();
            let per_page = get("per_page").and_then(|v| v.parse().ok()).unwrap_or(30usize);
            let page = get("page").and_then(|v| v.parse().ok()).unwrap_or(1usize).max(1);
            let mut items: Vec<Value> = repo
                .issues
                .iter()
                .enumerate()
                .filter(|(_, issue)| state == "all" || (state == "closed") != issue.open)
                .filter(|(_, issue)| issue.updated_at.as_str() >= since)
                .map(|(i, issue)| issue_json(i + 1, issue))
                .collect();
            if get("direction") != Some("asc") {
                items.reverse();
            }
            reply(StatusCode::OK, Value::Array(items.into_iter().skip((page - 1) * per_page).take(per_page).collect()))
        }
        ("GET", ["repos", _, _, "issues", _]) => match number_at(4, &repo) {
            Some(n) => reply(StatusCode::OK, issue_json(n, &repo.issues[n - 1])),
            None => not_found(),
        },
        ("POST", ["repos", _, _, "issues", _, "comments"]) => {
            let Some(n) = number_at(4, &repo) else { return not_found() };
            let Some(text) = sent["body"].as_str().filter(|t| !t.is_empty()) else {
                return reply(StatusCode::UNPROCESSABLE_ENTITY, json!({ "message": "Validation Failed" }));
            };
            let association = repo.hub_association.clone().unwrap_or_else(|| "NONE".into());
            let id = repo.comment(n as u64, HUB_LOGIN, &association, text, None).expect("the issue exists");
            let made = repo.issues[n - 1].comments.last().cloned().expect("just added");
            reply(StatusCode::CREATED, json!({ "id": id, "body": made.body, "created_at": made.created_at, "author_association": made.association, "user": { "login": made.login } }))
        }
        ("GET", ["repos", _, _, "issues", _, "comments"]) => {
            let Some(n) = number_at(4, &repo) else { return not_found() };
            let since = query.iter().find(|(k, _)| k == "since").map(|(_, v)| v.as_str()).unwrap_or_default();
            let found: Vec<Value> = repo.issues[n - 1].comments.iter().filter(|c| c.created_at.as_str() >= since).map(|c| json!({ "id": c.id, "body": c.body, "created_at": c.created_at, "author_association": c.association, "user": { "login": c.login } })).collect();
            reply(StatusCode::OK, Value::Array(found))
        }
        ("GET", ["repos", _, _, "issues", _, "timeline"]) => match number_at(4, &repo) {
            Some(n) => reply(StatusCode::OK, Value::Array(repo.issues[n - 1].timeline.clone())),
            None => not_found(),
        },
        ("GET", ["user"]) => reply(StatusCode::OK, json!({ "login": HUB_LOGIN })),
        ("GET", ["repos", _, _, "releases"]) => reply(StatusCode::OK, Value::Array(repo.releases.clone())),
        ("GET", ["search", "issues"]) => {
            let q = query.iter().find(|(k, _)| k == "q").map(|(_, v)| v.to_lowercase()).unwrap_or_default();
            let per_page = query.iter().find(|(k, _)| k == "per_page").and_then(|(_, v)| v.parse().ok()).unwrap_or(30usize);
            let blind = repo.search_blind || repo.blind_searches > 0;
            repo.blind_searches = repo.blind_searches.saturating_sub(1);
            let mut words = vec![];
            let (mut want_open, mut want_closed, mut want_issue) = (false, false, false);
            for word in q.split_whitespace() {
                match word {
                    "is:issue" => want_issue = true,
                    "is:open" => want_open = true,
                    "is:closed" => want_closed = true,
                    _ if word.contains(':') => {}
                    _ => words.push(word.trim_matches('"')),
                }
            }
            let items: Vec<Value> = repo
                .issues
                .iter()
                .enumerate()
                .filter(|_| !blind)
                .filter(|(_, issue)| (!want_open || issue.open) && (!want_closed || !issue.open) && !(want_issue && issue.pull))
                .filter(|(_, issue)| {
                    let text = format!("{} {}", issue.title, issue.body).to_lowercase();
                    words.iter().all(|word| text.contains(word))
                })
                .take(per_page)
                .map(|(i, issue)| issue_json(i + 1, issue))
                .collect();
            reply(StatusCode::OK, json!({ "total_count": items.len(), "items": items }))
        }
        _ => not_found(),
    }
}
