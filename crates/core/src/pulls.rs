//! The pull requests of a machine's projects, as T3 Code's Pull requests page lists them: one
//! GitHub search per batch of repositories, through gh's GraphQL API.

use crate::workspace::{run, Failure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// GitHub rejects a search query past 256 characters.
const QUERY_LIMIT: usize = 250;
const SEARCH: &str = r#"query($q: String!) {
  viewer { login }
  search(query: $q, type: ISSUE, first: 100) {
    nodes { ... on PullRequest {
      number title url isDraft state mergeable reviewDecision additions deletions createdAt updatedAt headRefName baseRefName
      author { login avatarUrl }
      repository { nameWithOwner }
      labels(first: 20) { nodes { name color } }
      reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login } ... on Team { slug } } } }
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
    } }
  }
}"#;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ListState {
    Open,
    Closed,
    Merged,
    All,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Involvement {
    All,
    Reviewing,
    Authored,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct PullRequests {
    /// The gh account's login.
    pub viewer: String,
    /// `owner/name` of each GitHub repository searched.
    pub repos: Vec<String>,
    pub rows: Vec<Row>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Row {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub repo: String,
    pub author: String,
    pub avatar: String,
    pub state: State,
    pub conflicting: bool,
    pub review: Option<Review>,
    pub checks: Option<Checks>,
    pub additions: u64,
    pub deletions: u64,
    pub created_at: String,
    pub updated_at: String,
    pub head: String,
    pub base: String,
    pub labels: Vec<Label>,
    pub group: Group,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Label {
    pub name: String,
    pub color: String,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Review {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Checks {
    Passing,
    Failing,
    Pending,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Authored,
    Reviewing,
    Others,
}

/// The search terms before the repositories, in T3 Code's order.
fn terms(state: ListState, involvement: Involvement, text: &str) -> String {
    let state = match state {
        ListState::Open => "is:open",
        ListState::Closed => "is:closed is:unmerged",
        ListState::Merged => "is:merged",
        ListState::All => "",
    };
    let involvement = match involvement {
        Involvement::Authored => "author:@me",
        Involvement::Reviewing => "review-requested:@me",
        Involvement::All => "",
    };
    ["is:pr", state, involvement, text.trim(), "sort:updated-desc"].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
}

/// `terms` with as many `repo:` qualifiers as fit under GitHub's query limit, per query.
fn queries(terms: &str, repos: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for repo in repos {
        let qualifier = format!(" repo:{repo}");
        match out.last_mut() {
            Some(q) if q.len() + qualifier.len() <= QUERY_LIMIT => q.push_str(&qualifier),
            _ => out.push(format!("{terms}{qualifier}")),
        }
    }
    out
}

/// `owner/name` of a GitHub remote URL, in any of its forms.
pub fn github_repo(remote: &str) -> Option<String> {
    let url = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = url.split_once("://").map_or_else(|| url.replacen(':', "/", 1), |(_, r)| r.to_owned());
    let rest = rest.rsplit_once('@').map_or(rest.as_str(), |(_, r)| r);
    let mut parts = rest.split('/');
    let host = parts.next()?.split(':').next()?;
    let (owner, name) = (parts.next()?, parts.next()?);
    (host.eq_ignore_ascii_case("github.com") && !owner.is_empty() && !name.is_empty() && parts.next().is_none()).then(|| format!("{owner}/{name}"))
}

fn search(cwd: &Path, path: Option<&str>, q: &str) -> Result<Value, String> {
    let out = run(cwd, path, "gh", &["api", "graphql", "-f", &format!("query={SEARCH}"), "-F", &format!("q={q}")]).map_err(|e| match e {
        Failure::Missing => "Install the GitHub command-line tool (`gh`) via https://cli.github.com/ or your package manager (for example `brew install gh`).".to_owned(),
        Failure::Said(said) if said.contains("auth login") || said.contains("not logged") => "GitHub is not authenticated. Run `gh auth login` (or set GH_TOKEN) and retry.".to_owned(),
        Failure::Said(said) => format!("gh: {said}"),
    })?;
    let doc: Value = serde_json::from_str(&out).map_err(|e| format!("gh sent a bad answer: {e}"))?;
    if let Some(message) = doc["errors"][0]["message"].as_str() {
        return Err(format!("GitHub: {message}"));
    }
    Ok(doc["data"].clone())
}

fn row(node: &Value, viewer: &str, reviewing: &HashSet<String>) -> Option<Row> {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let url = node["url"].as_str()?.to_owned();
    let author = s(&node["author"]["login"]);
    let state = match node["state"].as_str()? {
        "MERGED" => State::Merged,
        "CLOSED" => State::Closed,
        _ if node["isDraft"].as_bool() == Some(true) => State::Draft,
        _ => State::Open,
    };
    let review = match node["reviewDecision"].as_str() {
        Some("APPROVED") => Some(Review::Approved),
        Some("CHANGES_REQUESTED") => Some(Review::ChangesRequested),
        Some("REVIEW_REQUIRED") => Some(Review::ReviewRequired),
        _ => None,
    };
    let checks = match node["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["state"].as_str() {
        Some("SUCCESS") => Some(Checks::Passing),
        Some("FAILURE" | "ERROR") => Some(Checks::Failing),
        Some("PENDING" | "EXPECTED") => Some(Checks::Pending),
        _ => None,
    };
    let requested = node["reviewRequests"]["nodes"].as_array().into_iter().flatten().any(|r| r["requestedReviewer"]["login"].as_str() == Some(viewer));
    let group = if !viewer.is_empty() && author == viewer {
        Group::Authored
    } else if requested || reviewing.contains(&url) {
        Group::Reviewing
    } else {
        Group::Others
    };
    Some(Row {
        number: node["number"].as_u64()?,
        title: s(&node["title"]),
        repo: s(&node["repository"]["nameWithOwner"]),
        avatar: s(&node["author"]["avatarUrl"]),
        state,
        conflicting: node["mergeable"].as_str() == Some("CONFLICTING"),
        review,
        checks,
        additions: node["additions"].as_u64().unwrap_or(0),
        deletions: node["deletions"].as_u64().unwrap_or(0),
        created_at: s(&node["createdAt"]),
        updated_at: s(&node["updatedAt"]),
        head: s(&node["headRefName"]),
        base: s(&node["baseRefName"]),
        labels: node["labels"]["nodes"].as_array().into_iter().flatten().map(|l| Label { name: s(&l["name"]), color: s(&l["color"]) }).collect(),
        author,
        url,
        group,
    })
}

/// The pull requests of `repos` that match, newest update first. With every involvement, a second
/// search finds the ones awaiting the viewer's review, a team's request included.
pub fn list(cwd: &Path, path: Option<&str>, repos: Vec<String>, state: ListState, involvement: Involvement, text: &str) -> Result<PullRequests, String> {
    let mut viewer = String::new();
    let mut nodes = vec![];
    for q in queries(&terms(state, involvement, text), &repos) {
        let data = search(cwd, path, &q)?;
        viewer = data["viewer"]["login"].as_str().unwrap_or_default().to_owned();
        nodes.extend(data["search"]["nodes"].as_array().cloned().unwrap_or_default());
    }
    let mut reviewing = HashSet::new();
    if involvement == Involvement::All {
        for q in queries(&terms(state, Involvement::Reviewing, text), &repos) {
            let data = search(cwd, path, &q)?;
            reviewing.extend(data["search"]["nodes"].as_array().into_iter().flatten().filter_map(|n| n["url"].as_str().map(str::to_owned)));
        }
    }
    let mut seen = HashSet::new();
    let mut rows: Vec<Row> = nodes.iter().filter_map(|n| row(n, &viewer, &reviewing)).filter(|r| seen.insert(r.url.clone())).collect();
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(PullRequests { viewer, repos, rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_t3s_search_terms() {
        assert_eq!(terms(ListState::Open, Involvement::All, ""), "is:pr is:open sort:updated-desc");
        assert_eq!(terms(ListState::Closed, Involvement::Authored, " fix "), "is:pr is:closed is:unmerged author:@me fix sort:updated-desc");
        assert_eq!(terms(ListState::All, Involvement::Reviewing, ""), "is:pr review-requested:@me sort:updated-desc");
    }

    #[test]
    fn splits_repositories_across_queries_under_the_limit() {
        let repos: Vec<String> = (0..20).map(|i| format!("an-owner/repository-{i:02}")).collect();
        let qs = queries("is:pr is:open sort:updated-desc", &repos);
        assert!(qs.len() > 1);
        assert!(qs.iter().all(|q| q.len() <= QUERY_LIMIT && q.starts_with("is:pr is:open sort:updated-desc repo:")));
        assert_eq!(qs.iter().map(|q| q.matches(" repo:").count()).sum::<usize>(), 20);
        assert!(queries("is:pr", &[]).is_empty());
    }

    #[test]
    fn reads_github_remotes_only() {
        assert_eq!(github_repo("https://github.com/example/alpha.git").as_deref(), Some("example/alpha"));
        assert_eq!(github_repo("git@github.com:Example/Alpha.git").as_deref(), Some("Example/Alpha"));
        assert_eq!(github_repo("ssh://git@github.com/o/r").as_deref(), Some("o/r"));
        assert_eq!(github_repo("https://gitlab.com/o/r.git"), None);
    }

    #[test]
    fn maps_a_search_node() {
        let node = json!({
            "number": 7, "title": "Fix it", "url": "https://github.com/o/r/pull/7", "isDraft": false, "state": "OPEN", "mergeable": "CONFLICTING",
            "reviewDecision": "CHANGES_REQUESTED", "additions": 3, "deletions": 1, "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-02T00:00:00Z",
            "headRefName": "fix", "baseRefName": "main", "author": { "login": "someone", "avatarUrl": "https://avatars/x" }, "repository": { "nameWithOwner": "o/r" },
            "labels": { "nodes": [{ "name": "bug", "color": "d73a4a" }] },
            "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "me" } }] },
            "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "FAILURE" } } }] },
        });
        let r = row(&node, "me", &HashSet::new()).unwrap();
        assert_eq!((r.state, r.conflicting, r.review, r.checks, r.group), (State::Open, true, Some(Review::ChangesRequested), Some(Checks::Failing), Group::Reviewing));
        assert_eq!(r.labels, vec![Label { name: "bug".into(), color: "d73a4a".into() }]);
        let mine = row(&node, "someone", &HashSet::new()).unwrap();
        assert_eq!(mine.group, Group::Authored);
        let team = row(&json!({ "url": "https://github.com/o/r/pull/8", "number": 8, "state": "MERGED" }), "me", &["https://github.com/o/r/pull/8".to_owned()].into()).unwrap();
        assert_eq!((team.state, team.group), (State::Merged, Group::Reviewing));
    }
}
