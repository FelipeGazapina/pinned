//! The hub's calls to GitHub for the issues users report from the app, signed in with the hub's
//! own token, not a user's. Tests and the dev stack point it at `fake_github`.

use pinned_core::github::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

/// What the hub calls itself to GitHub.
const AGENT: &str = "pinned-hub";
/// Most hits `search_open` asks for.
const SEARCH_LIMIT: usize = 5;

/// What a send that may have reached GitHub says, whatever went wrong.
pub const MAY_HAVE_FILED: &str = "GitHub may have filed this report. Check My reports before trying again.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// GitHub answered 401, or 403 for a reason other than a rate limit, to the hub's token. A rate limit is `Busy`.
    Unauthorized,
    /// The outcome is unknown: the connection broke once the request was out, or the answer did not read.
    Failed(String),
    /// GitHub answered a rate limit or a 5xx: it did nothing.
    Busy(String),
    /// GitHub answered `status`, another refusal (a 404, a 422): it did nothing.
    Refused { status: u16, text: String },
    /// The request never left the hub.
    Unreachable(String),
    /// No answer in time: GitHub may have done it.
    TimedOut(String),
}

impl Error {
    /// Whether GitHub may have done what was asked, so a send that ended so counts toward the daily limit.
    pub fn may_have_reached(&self) -> bool {
        matches!(self, Error::Failed(_) | Error::TimedOut(_))
    }

    /// What a person or an agent reads of the failure: never GitHub's own words, which stay in the hub's log.
    pub fn public(&self) -> &'static str {
        match self {
            Error::Unauthorized | Error::Refused { .. } => "GitHub refused the report.",
            Error::Busy(_) => "GitHub is busy. Try again later.",
            Error::Failed(_) | Error::Unreachable(_) => "Could not reach GitHub.",
            Error::TimedOut(_) => "GitHub took too long to answer.",
        }
    }

    /// What a person or an agent reads of a send that ended so: `public`, unless GitHub may have filed it, when they
    /// look before they try again.
    pub fn public_send(&self) -> &'static str {
        match self.may_have_reached() {
            true => MAY_HAVE_FILED,
            false => self.public(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unauthorized => f.write_str("GitHub refused the hub's token"),
            Error::Failed(text) | Error::Busy(text) | Error::Refused { text, .. } | Error::Unreachable(text) | Error::TimedOut(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for Error {}

impl From<pinned_core::github::Error> for Error {
    fn from(error: pinned_core::github::Error) -> Error {
        use pinned_core::github::Error as Core;
        match error {
            Core::Unauthorized(_) => Error::Unauthorized,
            Core::Failed(text) => Error::Failed(text),
            Core::Answered { busy: true, text, .. } => Error::Busy(text),
            Core::Answered { status, text, .. } => Error::Refused { status, text },
            Core::Unreachable(text) => Error::Unreachable(text),
            Core::TimedOut(text) => Error::TimedOut(text),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub html_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IssueInfo {
    pub number: u64,
    /// `open` or `closed`.
    pub state: String,
    /// `completed`, `not_planned`, `duplicate` or `reopened` once GitHub says why.
    #[serde(default)]
    pub state_reason: Option<String>,
    pub title: String,
    pub html_url: String,
    #[serde(default)]
    pub comments: u64,
    #[serde(default)]
    pub closed_at: Option<String>,
    /// Set when only collaborators may comment.
    #[serde(default)]
    pub locked: bool,
    /// Present when the number is a pull request's.
    #[serde(default)]
    pub pull_request: Option<Value>,
}

/// An issue as the poller reads it: where it stands, how many comments it has and its label names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracked {
    pub number: u64,
    pub open: bool,
    /// `completed`, `not_planned`, `duplicate`, `reopened`, ... as GitHub says; anything may come.
    pub state_reason: Option<String>,
    pub comments: u64,
    pub labels: Vec<String>,
}

fn tracked(item: &Value) -> Option<Tracked> {
    Some(Tracked {
        number: item["number"].as_u64()?,
        open: item["state"].as_str()? == "open",
        state_reason: text(&item["state_reason"]),
        comments: item["comments"].as_u64().unwrap_or_default(),
        labels: item["labels"].as_array().map(|l| l.iter().filter_map(|l| text(&l["name"])).collect()).unwrap_or_default(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Hit {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    #[serde(default)]
    pub comments: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: u64,
    pub created_at: String,
    pub user_login: String,
    /// GitHub's `author_association`: `OWNER`, `MEMBER`, `COLLABORATOR`, `CONTRIBUTOR`, `NONE`, ...
    pub author_association: String,
}

/// A pull request that mentioned the issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRef {
    pub number: u64,
    /// Set once the pull request is merged.
    pub merged_at: Option<String>,
}

/// One entry of an issue's timeline, as far as the hub reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEvent {
    /// `closed`, `cross-referenced`, `commented`, ...
    pub event: String,
    pub created_at: Option<String>,
    /// On `closed`: the commit that closed the issue, if one did.
    pub commit_id: Option<String>,
    /// On `cross-referenced`: the pull request that mentioned the issue.
    pub pull: Option<PullRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub tag_name: String,
    /// Absent while a release is a draft.
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
}

/// One repository on one GitHub API, as the hub's token sees it.
#[derive(Clone)]
pub struct Github {
    api: String,
    repo: String,
    token: String,
    timeout: std::time::Duration,
    /// The login of the hub's token, once `GET /user` said it.
    own_login: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

// Not derived, so the token never reaches a log.
impl std::fmt::Debug for Github {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Github").field("api", &self.api).field("repo", &self.repo).finish_non_exhaustive()
    }
}

/// Whether issue or comment `item` is the hub's, by `login`, and its body carries the footer reference `report_ref`.
/// A pull request never is: anyone may copy a footer into one.
fn carries_ref(item: &Value, report_ref: &str, login: &str) -> bool {
    item["pull_request"].is_null()
        && item["user"]["login"].as_str().is_some_and(|author| author.eq_ignore_ascii_case(login))
        && item["body"].as_str().is_some_and(|body| body.contains(&format!("report {report_ref}")))
}

/// The oldest of `items` that `carries_ref`: GitHub numbers issues in the order it makes them.
fn oldest_with_ref<'a>(items: &'a [Value], report_ref: &str, login: &str) -> Option<&'a Value> {
    items.iter().filter(|item| carries_ref(item, report_ref, login)).min_by_key(|item| item["number"].as_u64().unwrap_or(u64::MAX))
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|e| Error::Failed(format!("GitHub sent an unexpected answer: {e}")))
}

fn decode_list<T: DeserializeOwned>(value: Value) -> Result<Vec<T>> {
    match value {
        Value::Array(items) => items.into_iter().map(decode).collect(),
        _ => Err(Error::Failed("GitHub sent an unexpected answer: not a list".into())),
    }
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

impl Github {
    /// `repo` (`owner/name`) on the real GitHub.
    pub fn new(repo: &str, token: &str) -> Github {
        Github::with_api(&Client::new("").api, repo, token)
    }

    /// `repo` on another API root, such as `fake_github`'s.
    pub fn with_api(api: &str, repo: &str, token: &str) -> Github {
        Github { api: api.trim_end_matches('/').to_owned(), repo: repo.to_owned(), token: token.to_owned(), timeout: pinned_core::github::CALL_TIMEOUT, own_login: Default::default() }
    }

    /// This client with calls that give up after `timeout`.
    pub fn with_timeout(self, timeout: std::time::Duration) -> Github {
        Github { timeout, ..self }
    }

    /// How long one call may take.
    pub fn timeout(&self) -> std::time::Duration {
        self.timeout
    }

    /// The login of the hub's token, asked of GitHub once.
    async fn own_login(&self) -> Result<String> {
        if let Some(login) = self.own_login.lock().unwrap().clone() {
            return Ok(login);
        }
        let user = self.get("/user").await?;
        let login = user["login"].as_str().filter(|login| !login.is_empty()).ok_or_else(|| Error::Failed("GitHub sent a user without a login".into()))?.to_owned();
        *self.own_login.lock().unwrap() = Some(login.clone());
        Ok(login)
    }

    /// `name` of the repository `owner/name`.
    pub fn repo_name(&self) -> &str {
        self.repo.rsplit('/').next().unwrap_or(&self.repo)
    }

    fn client(&self) -> Client {
        Client { api: self.api.clone(), token: self.token.clone(), timeout: self.timeout }
    }

    async fn get(&self, path: &str) -> Result<Value> {
        Ok(self.client().get_as(AGENT, path).await?)
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        Ok(self.client().post_as(AGENT, path, body).await?)
    }

    pub async fn create_issue(&self, title: &str, body: &str, labels: &[String]) -> Result<Issue> {
        decode(self.post(&format!("/repos/{}/issues", self.repo), &json!({ "title": title, "body": body, "labels": labels })).await?)
    }

    /// The id of the comment it adds.
    pub async fn add_comment(&self, number: u64, body: &str) -> Result<u64> {
        let comment = self.post(&format!("/repos/{}/issues/{number}/comments", self.repo), &json!({ "body": body })).await?;
        comment["id"].as_u64().ok_or_else(|| Error::Failed("GitHub sent a comment without an id".into()))
    }

    pub async fn issue(&self, number: u64) -> Result<IssueInfo> {
        decode(self.get(&format!("/repos/{}/issues/{number}", self.repo)).await?)
    }

    /// Up to five open issues whose title or body matches `query`.
    pub async fn search_open(&self, query: &str) -> Result<Vec<Hit>> {
        let q = format!("{query} repo:{} is:issue is:open in:title,body", self.repo);
        let q: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
        let mut found = self.get(&format!("/search/issues?q={q}&per_page={SEARCH_LIMIT}")).await?;
        decode_list(found["items"].take())
    }

    /// The issue, open or closed, that the hub's token made and whose body carries `report_ref`, the footer
    /// reference of a report first sent at `sent_at`; the oldest, when a copy of the footer is about. It looks first
    /// in the issues changed since a minute before then, which GitHub answers at once; only when they hold none does
    /// it ask search, whose index lags.
    pub async fn issue_with_ref(&self, report_ref: &str, sent_at: Option<i64>) -> Result<Option<Issue>> {
        let login = self.own_login().await?;
        if let Some(sent_at) = sent_at {
            let since: String = url::form_urlencoded::byte_serialize(super::poller::iso(sent_at - 60).as_bytes()).collect();
            let mut found: Vec<Value> = vec![];
            for page in 1..=10 {
                let listed = self.get(&format!("/repos/{}/issues?state=all&since={since}&sort=created&direction=desc&per_page=100&page={page}", self.repo)).await?;
                let Value::Array(items) = listed else { return Err(Error::Failed("GitHub sent an unexpected answer: not a list".into())) };
                let full = items.len() == 100;
                found.extend(items.into_iter().filter(|item| carries_ref(item, report_ref, &login)));
                if !full {
                    break;
                }
            }
            if let Some(item) = oldest_with_ref(&found, report_ref, &login) {
                return decode(item.clone()).map(Some);
            }
        }
        let q = format!("\"{report_ref}\" repo:{} is:issue in:body", self.repo);
        let q: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
        let mut found = self.get(&format!("/search/issues?q={q}&per_page={SEARCH_LIMIT}")).await?;
        let Value::Array(items) = found["items"].take() else { return Err(Error::Failed("GitHub sent an unexpected answer: not a list".into())) };
        oldest_with_ref(&items, report_ref, &login).cloned().map(decode).transpose()
    }

    /// The id of the comment on issue `number` that the hub's token wrote and whose body carries `report_ref`; the
    /// first such, as GitHub lists comments oldest first.
    pub async fn comment_with_ref(&self, number: u64, report_ref: &str) -> Result<Option<u64>> {
        let login = self.own_login().await?;
        for page in 1..=10 {
            let found = self.get(&format!("/repos/{}/issues/{number}/comments?per_page=100&page={page}", self.repo)).await?;
            let Value::Array(items) = found else { return Err(Error::Failed("GitHub sent an unexpected answer: not a list".into())) };
            if let Some(comment) = items.iter().find(|c| carries_ref(c, report_ref, &login)) {
                return comment["id"].as_u64().map(Some).ok_or_else(|| Error::Failed("GitHub sent a comment without an id".into()));
            }
            if items.len() < 100 {
                break;
            }
        }
        Ok(None)
    }

    /// One page (`per_page` issues) of the issues changed at or after `since_iso`. The query holds nothing else,
    /// so no word of any report goes to GitHub's search.
    pub async fn search_updated(&self, since_iso: &str, per_page: usize, page: u32) -> Result<Vec<Tracked>> {
        let q = format!("repo:{} is:issue updated:>={since_iso}", self.repo);
        let q: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
        let found = self.get(&format!("/search/issues?q={q}&per_page={per_page}&page={page}")).await?;
        let Some(items) = found["items"].as_array() else { return Err(Error::Failed("GitHub sent an unexpected answer: no items".into())) };
        Ok(items.iter().filter_map(tracked).collect())
    }

    /// One issue as the poller reads it; None when GitHub has no such issue any more.
    pub async fn tracked(&self, number: u64) -> Result<Option<Tracked>> {
        match self.get(&format!("/repos/{}/issues/{number}", self.repo)).await {
            Ok(item) => Ok(tracked(&item)),
            Err(Error::Refused { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The comments written at or after `since_iso`, oldest first.
    pub async fn comments_since(&self, number: u64, since_iso: &str) -> Result<Vec<Comment>> {
        let since: String = url::form_urlencoded::byte_serialize(since_iso.as_bytes()).collect();
        let found = self.get(&format!("/repos/{}/issues/{number}/comments?since={since}&per_page=100", self.repo)).await?;
        let Value::Array(items) = found else { return Err(Error::Failed("GitHub sent an unexpected answer: not a list".into())) };
        items
            .iter()
            .map(|c| {
                Ok(Comment {
                    id: c["id"].as_u64().ok_or_else(|| Error::Failed("GitHub sent a comment without an id".into()))?,
                    created_at: text(&c["created_at"]).unwrap_or_default(),
                    user_login: text(&c["user"]["login"]).unwrap_or_default(),
                    author_association: text(&c["author_association"]).unwrap_or_default(),
                })
            })
            .collect()
    }

    /// What closed the issue and which pull requests mentioned it.
    pub async fn timeline(&self, number: u64) -> Result<Vec<TimelineEvent>> {
        let found = self.get(&format!("/repos/{}/issues/{number}/timeline?per_page=100", self.repo)).await?;
        let Value::Array(items) = found else { return Err(Error::Failed("GitHub sent an unexpected answer: not a list".into())) };
        Ok(items
            .iter()
            .map(|e| {
                let issue = &e["source"]["issue"];
                let pull = issue["pull_request"].is_object().then(|| PullRef { number: issue["number"].as_u64().unwrap_or_default(), merged_at: text(&issue["pull_request"]["merged_at"]) });
                TimelineEvent { event: text(&e["event"]).unwrap_or_default(), created_at: text(&e["created_at"]), commit_id: text(&e["commit_id"]), pull }
            })
            .collect())
    }

    pub async fn releases(&self) -> Result<Vec<Release>> {
        decode_list(self.get(&format!("/repos/{}/releases?per_page=100", self.repo)).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake_github::{self, FakeGithub};
    use super::*;

    const REPO: &str = "acme/app";
    const TOKEN: &str = "fake-secret-token-1";

    async fn setup() -> (FakeGithub, Github) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let fake = fake_github::start().await;
        let github = Github::with_api(&fake.url, REPO, TOKEN);
        (fake, github)
    }

    fn labels(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_send_that_may_have_reached_github_says_to_check_my_reports_and_any_other_keeps_its_text() {
        let may = "GitHub may have filed this report. Check My reports before trying again.";
        assert_eq!(Error::TimedOut("no answer in time".into()).public_send(), may);
        assert_eq!(Error::Failed("GitHub sent a bad response".into()).public_send(), may);
        assert_eq!(Error::Unreachable("connection refused".into()).public_send(), "Could not reach GitHub.");
        assert_eq!(Error::Busy("rate limit".into()).public_send(), "GitHub is busy. Try again later.");
        assert_eq!(Error::Refused { status: 422, text: "Validation Failed".into() }.public_send(), "GitHub refused the report.");
        assert_eq!(Error::Unauthorized.public_send(), "GitHub refused the report.");
    }

    #[tokio::test]
    async fn an_issue_is_created_with_its_labels_and_read_back() {
        let (fake, github) = setup().await;
        let issue = github.create_issue("Crash on launch", "It dies at startup.", &labels(&["bug", "from-app"])).await.unwrap();
        assert_eq!(issue, Issue { number: 1, html_url: "https://github.com/acme/app/issues/1".into() });
        let info = github.issue(1).await.unwrap();
        assert_eq!(
            info,
            IssueInfo { number: 1, state: "open".into(), state_reason: None, title: "Crash on launch".into(), html_url: "https://github.com/acme/app/issues/1".into(), comments: 0, closed_at: None, locked: false, pull_request: None }
        );
        assert_eq!(fake.labels(1), ["bug", "from-app"]);
        assert_eq!(fake.body(1), "It dies at startup.");
        assert_eq!(github.create_issue("Second", "", &[]).await.unwrap().number, 2);
        assert!(matches!(github.issue(9).await, Err(Error::Refused { status: 404, text }) if text.contains("404")));
    }

    #[tokio::test]
    async fn a_comment_gets_an_id_and_is_counted() {
        let (fake, github) = setup().await;
        github.create_issue("Slow sync", "Takes minutes.", &[]).await.unwrap();
        assert_eq!(github.add_comment(1, "Same here").await.unwrap(), 1);
        assert_eq!(github.add_comment(1, "And here").await.unwrap(), 2);
        assert_eq!(github.issue(1).await.unwrap().comments, 2);
        assert_eq!(fake.comment_bodies(1), ["Same here", "And here"]);
    }

    #[tokio::test]
    async fn search_finds_an_open_issue_by_a_title_word_and_not_a_closed_one() {
        let (fake, github) = setup().await;
        github.create_issue("Crash on launch", "It dies at startup.", &[]).await.unwrap();
        github.create_issue("Slow sync", "Syncing takes minutes.", &[]).await.unwrap();
        github.create_issue("Launch button missing", "No button.", &[]).await.unwrap();
        let hits = github.search_open("launch").await.unwrap();
        assert_eq!(
            hits,
            [
                Hit { number: 1, title: "Crash on launch".into(), html_url: "https://github.com/acme/app/issues/1".into(), comments: 0 },
                Hit { number: 3, title: "Launch button missing".into(), html_url: "https://github.com/acme/app/issues/3".into(), comments: 0 },
            ]
        );
        assert_eq!(github.search_open("minutes").await.unwrap().iter().map(|h| h.number).collect::<Vec<_>>(), [2]);
        fake.close(1, "completed", None);
        assert_eq!(github.search_open("launch").await.unwrap().iter().map(|h| h.number).collect::<Vec<_>>(), [3]);
        assert!(github.search_open("nothing like it").await.unwrap().is_empty());
        let logged = fake.requests();
        let search = logged.iter().find(|r| r.path.starts_with("/search/issues")).unwrap();
        assert_eq!(search.method, "GET");
        assert_eq!(search.path, "/search/issues?q=launch+repo%3Aacme%2Fapp+is%3Aissue+is%3Aopen+in%3Atitle%2Cbody&per_page=5");
    }

    #[tokio::test]
    async fn search_returns_at_most_five_hits() {
        let (_fake, github) = setup().await;
        for n in 0..7 {
            github.create_issue(&format!("Crash {n}"), "", &[]).await.unwrap();
        }
        assert_eq!(github.search_open("crash").await.unwrap().len(), 5);
    }

    #[tokio::test]
    async fn comments_since_lists_the_later_ones_with_their_author() {
        let (fake, github) = setup().await;
        github.create_issue("Crash on launch", "", &[]).await.unwrap();
        assert_eq!(fake.add_comment_as(1, "bob", "NONE", "first"), 1);
        fake.add_comment_at(1, "alice", "second", "2026-03-01T10:00:00Z");
        fake.add_comment_at(1, "maintainer-bot", "third", "2026-03-02T10:00:00Z");
        let later = github.comments_since(1, "2026-03-01T12:00:00Z").await.unwrap();
        assert_eq!(later, [Comment { id: 3, created_at: "2026-03-02T10:00:00Z".into(), user_login: "maintainer-bot".into(), author_association: "NONE".into() }]);
        let both = github.comments_since(1, "2026-03-01T00:00:00Z").await.unwrap();
        assert_eq!(both.iter().map(|c| (c.id, c.user_login.as_str())).collect::<Vec<_>>(), [(2, "alice"), (3, "maintainer-bot")]);
        let all = github.comments_since(1, "2025-01-01T00:00:00Z").await.unwrap();
        assert_eq!((all.len(), all[0].user_login.as_str(), all[0].created_at.as_str()), (3, "bob", "2026-01-01T00:00:01Z"));
        assert!(github.comments_since(1, "2999-01-01T00:00:00Z").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_comment_carries_the_author_association_github_gave_it() {
        let (fake, github) = setup().await;
        github.create_issue("Crash on launch", "", &[]).await.unwrap();
        fake.add_comment_as(1, "ana", "OWNER", "We will look.");
        fake.add_comment_as(1, "bob", "NONE", "Same here.");
        github.add_comment(1, "From the hub").await.unwrap();
        let all = github.comments_since(1, "2025-01-01T00:00:00Z").await.unwrap();
        assert_eq!(all.iter().map(|c| (c.user_login.as_str(), c.author_association.as_str())).collect::<Vec<_>>(), [("ana", "OWNER"), ("bob", "NONE"), ("pinned-hub", "NONE")]);
    }

    #[tokio::test]
    async fn a_403_for_a_rate_limit_is_a_temporary_failure_and_any_other_403_is_unauthorized() {
        let (fake, github) = setup().await;
        // The header says the limit is used up.
        fake.fail_next(403, "Forbidden", Some("0"));
        assert!(matches!(github.issue(1).await, Err(Error::Busy(text)) if text.contains("rate limit")));
        // The message says so.
        fake.fail_next(403, "API rate limit exceeded", None);
        assert!(matches!(github.create_issue("x", "y", &[]).await, Err(Error::Busy(text)) if text.contains("rate limit")));
        // Neither: the token may not do this.
        fake.fail_next(403, "Resource not accessible by personal access token", Some("4990"));
        assert_eq!(github.issue(1).await.unwrap_err(), Error::Unauthorized);
        // 401 stays unauthorized, and the failure was for one request only.
        assert!(github.create_issue("Real", "y", &[]).await.is_ok());
        let stranger = Github::with_api(&fake.url, REPO, "ghp_real_looking_token");
        assert_eq!(stranger.issue(1).await.unwrap_err(), Error::Unauthorized);
    }

    #[tokio::test]
    async fn the_timeline_names_the_closing_commit_and_the_merged_pull_request_and_releases_list() {
        let (fake, github) = setup().await;
        github.create_issue("Crash on launch", "", &[]).await.unwrap();
        fake.add_merged_pull(1, 42, "2026-04-01T09:30:00Z");
        fake.close(1, "completed", Some("abc123def"));
        fake.add_release("v1.0.0", "2026-03-01T00:00:00Z");
        fake.add_release("v1.1.0", "2026-04-02T12:00:00Z");
        let info = github.issue(1).await.unwrap();
        assert_eq!((info.state.as_str(), info.state_reason.as_deref()), ("closed", Some("completed")));
        assert!(info.closed_at.is_some());
        let timeline = github.timeline(1).await.unwrap();
        let closed = timeline.iter().find(|e| e.event == "closed").unwrap();
        assert_eq!(closed.commit_id.as_deref(), Some("abc123def"));
        let referenced = timeline.iter().find(|e| e.event == "cross-referenced").unwrap();
        assert_eq!(referenced.pull, Some(PullRef { number: 42, merged_at: Some("2026-04-01T09:30:00Z".into()) }));
        assert_eq!(
            github.releases().await.unwrap(),
            [
                Release { tag_name: "v1.0.0".into(), published_at: Some("2026-03-01T00:00:00Z".into()), draft: false, prerelease: false },
                Release { tag_name: "v1.1.0".into(), published_at: Some("2026-04-02T12:00:00Z".into()), draft: false, prerelease: false },
            ]
        );
    }

    #[tokio::test]
    async fn a_token_the_fake_does_not_know_is_unauthorized_on_every_call() {
        let (fake, _) = setup().await;
        let stranger = Github::with_api(&fake.url, REPO, "ghp_real_looking_token");
        assert_eq!(stranger.create_issue("x", "y", &[]).await.unwrap_err(), Error::Unauthorized);
        assert_eq!(stranger.issue(1).await.unwrap_err(), Error::Unauthorized);
        assert_eq!(stranger.search_open("x").await.unwrap_err(), Error::Unauthorized);
        assert_eq!(stranger.releases().await.unwrap_err(), Error::Unauthorized);
        assert_eq!(Error::Unauthorized.to_string(), "GitHub refused the hub's token");
        assert!(!format!("{stranger:?}").contains("ghp_real_looking_token"));
    }

    #[tokio::test]
    async fn the_request_log_never_holds_the_token_and_the_headers_are_the_hubs() {
        let (fake, github) = setup().await;
        github.create_issue("Crash on launch", "body", &labels(&["bug"])).await.unwrap();
        github.add_comment(1, "hi").await.unwrap();
        github.issue(1).await.unwrap();
        let stranger = Github::with_api(&fake.url, REPO, "ghp_other_secret");
        let _ = stranger.issue(1).await;
        let logged = fake.requests();
        assert_eq!(
            logged.iter().map(|r| (r.method.as_str(), r.path.as_str())).collect::<Vec<_>>(),
            [
                ("POST", "/repos/acme/app/issues"),
                ("POST", "/repos/acme/app/issues/1/comments"),
                ("GET", "/repos/acme/app/issues/1"),
                ("GET", "/repos/acme/app/issues/1"),
            ]
        );
        assert!(logged[0].body.contains("\"title\":\"Crash on launch\""));
        let everything = format!("{logged:?}");
        for secret in [TOKEN, "ghp_other_secret", "fake-secret", "Bearer"] {
            assert!(!everything.contains(secret), "the log holds {secret}");
        }
        assert_eq!(fake.last_headers(), vec![
            ("accept".to_owned(), "application/vnd.github+json".to_owned()),
            ("user-agent".to_owned(), "pinned-hub".to_owned()),
            ("x-github-api-version".to_owned(), "2022-11-28".to_owned()),
        ]);
    }
}
