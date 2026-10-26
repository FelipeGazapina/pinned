//! The pull requests of a machine's projects, as T3 Code's Pull requests page lists them: one
//! GitHub search per batch of repositories, through gh's GraphQL API.

use crate::workspace::{run, Failure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// GitHub rejects a search query past 256 characters.
const QUERY_LIMIT: usize = 250;
/// T3 Code loads at most this many rows.
pub const MAX_ROWS: usize = 500;
/// The search, with GitHub's pull request stacks on github.com: Enterprise Server has none.
fn search_query(stacks: bool) -> String {
    let stack = if stacks { "\n      stack { number size baseRefName } stackEntry { position }" } else { "" };
    format!(
        r#"query($q: String!, $after: String) {{
  viewer {{ login }}
  search(query: $q, type: ISSUE, first: 100, after: $after) {{
    pageInfo {{ hasNextPage endCursor }}
    nodes {{ ... on PullRequest {{
      number title url isDraft state mergeable reviewDecision additions deletions createdAt updatedAt headRefName baseRefName
      author {{ login avatarUrl }}
      repository {{ nameWithOwner }}
      labels(first: 20) {{ nodes {{ name color }} }}
      reviewRequests(first: 20) {{ nodes {{ requestedReviewer {{ ... on User {{ login }} ... on Team {{ slug }} }} }} }}
      commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ state }} }} }} }}{stack}
    }} }}
  }}
}}"#
    )
}

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

pub const GITHUB: &str = "github.com";

/// A GitHub repository: `owner/name` on `host`, github.com or an Enterprise Server's.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Repo {
    pub host: String,
    pub name: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct PullRequests {
    /// The gh account's login.
    pub viewer: String,
    /// Each GitHub repository searched.
    pub repos: Vec<Repo>,
    pub rows: Vec<Row>,
    /// GitHub has more past the `limit` asked for.
    pub more: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Row {
    pub host: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<StackMembership>,
}

/// Where a pull request sits in one of GitHub's pull request stacks.
#[derive(Serialize, Debug, PartialEq)]
pub struct StackMembership {
    pub number: u64,
    pub position: u64,
    pub size: u64,
    pub base: String,
}

/// A GitHub pull request stack, its layers from the bottom up.
#[derive(Serialize, Debug, PartialEq)]
pub struct Stack {
    pub number: u64,
    pub url: String,
    pub base: String,
    pub layers: Vec<Layer>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Layer {
    pub number: u64,
    pub title: String,
    pub head: String,
    pub state: State,
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

/// The host, lowercase, and `owner/name` of a git remote URL, in any of its forms.
pub fn remote_repo(remote: &str) -> Option<Repo> {
    let url = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = url.split_once("://").map_or_else(|| url.replacen(':', "/", 1), |(_, r)| r.to_owned());
    let rest = rest.rsplit_once('@').map_or(rest.as_str(), |(_, r)| r);
    let mut parts = rest.split('/');
    let host = parts.next()?.split(':').next()?.to_ascii_lowercase();
    let (owner, name) = (parts.next()?, parts.next()?);
    (!host.is_empty() && !owner.is_empty() && !name.is_empty() && parts.next().is_none()).then(|| Repo { host, name: format!("{owner}/{name}") })
}

/// Whether gh can reach `host`: github.com always; another host once `gh auth login` added it,
/// which is how an Enterprise Server is told apart from GitLab and the rest.
pub fn gh_knows(cwd: &Path, path: Option<&str>, host: &str) -> bool {
    host == GITHUB || run(cwd, path, "gh", &["auth", "token", "--hostname", host]).is_ok()
}

fn gh_failure(e: Failure) -> String {
    match e {
        Failure::Missing => "Install the GitHub command-line tool (`gh`) via https://cli.github.com/ or your package manager (for example `brew install gh`).".to_owned(),
        Failure::Said(said) if said.contains("auth login") || said.contains("not logged") => "GitHub is not authenticated. Run `gh auth login` (or set GH_TOKEN) and retry.".to_owned(),
        Failure::Said(said) => format!("gh: {said}"),
    }
}

fn search(cwd: &Path, path: Option<&str>, host: &str, q: &str, after: Option<&str>) -> Result<Value, String> {
    let (query, q) = (format!("query={}", search_query(host == GITHUB)), format!("q={q}"));
    let after = after.map(|a| format!("after={a}"));
    let mut args = vec!["api", "graphql", "--hostname", host, "-f", &query, "-f", &q];
    if let Some(after) = &after {
        args.extend(["-f", after]);
    }
    let out = run(cwd, path, "gh", &args).map_err(gh_failure)?;
    let doc: Value = serde_json::from_str(&out).map_err(|e| format!("gh sent a bad answer: {e}"))?;
    if let Some(message) = doc["errors"][0]["message"].as_str() {
        return Err(format!("GitHub: {message}"));
    }
    Ok(doc["data"].clone())
}

fn row(host: &str, node: &Value, viewer: &str, reviewing: &HashSet<String>) -> Option<Row> {
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
    let stack = node["stack"]["number"].as_u64().zip(node["stackEntry"]["position"].as_u64()).map(|(number, position)| StackMembership {
        number,
        position,
        size: node["stack"]["size"].as_u64().unwrap_or(position),
        base: s(&node["stack"]["baseRefName"]),
    });
    Some(Row {
        host: host.to_owned(),
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
        stack,
    })
}

/// Up to `limit` nodes of the search `q`, a page of 100 at a time, the viewer's login, and whether more remain.
fn pages(cwd: &Path, path: Option<&str>, host: &str, q: &str, limit: usize) -> Result<(Vec<Value>, String, bool), String> {
    let (mut nodes, mut after) = (vec![], None::<String>);
    loop {
        let data = search(cwd, path, host, q, after.as_deref())?;
        nodes.extend(data["search"]["nodes"].as_array().cloned().unwrap_or_default());
        let viewer = data["viewer"]["login"].as_str().unwrap_or_default().to_owned();
        let info = &data["search"]["pageInfo"];
        let more = info["hasNextPage"].as_bool() == Some(true);
        if !more || nodes.len() >= limit {
            nodes.truncate(limit);
            return Ok((nodes, viewer, more));
        }
        after = info["endCursor"].as_str().map(str::to_owned);
    }
}

/// The pull requests of `repos` that match, newest update first, `limit` per search at most, one
/// host's searches at a time. With every involvement, a second search finds the ones awaiting the
/// viewer's review, a team's request included.
pub fn list(cwd: &Path, path: Option<&str>, repos: Vec<Repo>, state: ListState, involvement: Involvement, text: &str, limit: usize) -> Result<PullRequests, String> {
    let limit = limit.clamp(1, MAX_ROWS);
    let mut hosts: Vec<&str> = repos.iter().map(|r| r.host.as_str()).collect();
    hosts.dedup();
    let (mut viewer, mut rows, mut more) = (String::new(), vec![], false);
    let mut seen = HashSet::new();
    for host in hosts {
        let names: Vec<String> = repos.iter().filter(|r| r.host == host).map(|r| r.name.clone()).collect();
        let mut nodes = vec![];
        let mut login = String::new();
        for q in queries(&terms(state, involvement, text), &names) {
            let (found, who, left) = pages(cwd, path, host, &q, limit)?;
            (login, more) = (who, more || left);
            nodes.extend(found);
        }
        let mut reviewing = HashSet::new();
        if involvement == Involvement::All {
            for q in queries(&terms(state, Involvement::Reviewing, text), &names) {
                reviewing.extend(pages(cwd, path, host, &q, limit)?.0.iter().filter_map(|n| n["url"].as_str().map(str::to_owned)));
            }
        }
        rows.extend(nodes.iter().filter_map(|n| row(host, n, &login, &reviewing)).filter(|r| seen.insert(r.url.clone())));
        if host == GITHUB || viewer.is_empty() {
            viewer = login;
        }
    }
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(PullRequests { viewer, repos, rows, more })
}

fn layer_state(layer: &Value) -> State {
    match (layer["state"].as_str().unwrap_or_default(), layer["merged_at"].is_string()) {
        (_, true) => State::Merged,
        ("closed", _) => State::Closed,
        _ if layer["draft"].as_bool() == Some(true) => State::Draft,
        _ => State::Open,
    }
}

fn read_stack(raw: &Value) -> Option<Stack> {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    Some(Stack {
        number: raw["number"].as_u64()?,
        url: raw["html_url"].as_str().map_or_else(|| s(&raw["url"]), str::to_owned),
        base: raw["base"].as_str().map_or_else(|| s(&raw["base"]["ref"]), str::to_owned),
        layers: raw["pull_requests"].as_array().into_iter().flatten().filter_map(|l| Some(Layer { number: l["number"].as_u64()?, title: s(&l["title"]), head: s(&l["head"]["ref"]), state: layer_state(l) })).collect(),
    })
}

/// The stack pull request `number` of github.com's `repo` sits in, or `None` when it is in none,
/// or the repository has no stacks.
pub fn stack(cwd: &Path, path: Option<&str>, repo: &str, number: u64) -> Result<Option<Stack>, String> {
    let get = |route: String| match run(cwd, path, "gh", &["api", &route]) {
        Ok(out) => serde_json::from_str::<Value>(&out).map(Some).map_err(|e| format!("gh sent a bad answer: {e}")),
        Err(Failure::Said(said)) if said.contains("404") || said.contains("Not Found") => Ok(None),
        Err(e) => Err(gh_failure(e)),
    };
    let Some(found) = get(format!("repos/{repo}/stacks?pull_request={number}"))? else { return Ok(None) };
    let Some(number) = found[0]["number"].as_u64() else { return Ok(None) };
    Ok(get(format!("repos/{repo}/stacks/{number}"))?.as_ref().and_then(read_stack))
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
    fn reads_a_remotes_host_and_repository() {
        let repo = |host: &str, name: &str| Some(Repo { host: host.into(), name: name.into() });
        assert_eq!(remote_repo("https://github.com/example/alpha.git"), repo("github.com", "example/alpha"));
        assert_eq!(remote_repo("git@GitHub.com:Example/Alpha.git"), repo("github.com", "Example/Alpha"));
        assert_eq!(remote_repo("ssh://git@github.com/o/r"), repo("github.com", "o/r"));
        assert_eq!(remote_repo("https://ghe.acme.test/o/r.git"), repo("ghe.acme.test", "o/r"));
        assert_eq!(remote_repo("https://gitlab.com/group/sub/r.git"), None);
    }

    #[test]
    fn asks_for_stacks_on_github_com_only() {
        assert!(search_query(true).contains("stack { number size baseRefName } stackEntry { position }"));
        assert!(!search_query(false).contains("stack"));
    }

    #[test]
    fn reads_a_stack_bottom_layer_first() {
        let raw = json!({ "number": 7, "url": "https://api.github.com/repos/o/r/stacks/7", "html_url": "https://github.com/o/r/stacks/7", "base": { "ref": "main" }, "pull_requests": [
            { "number": 42, "title": "Add notes", "head": { "ref": "notes" }, "state": "open", "draft": false, "merged_at": null },
            { "number": 41, "title": "Draft more", "head": { "ref": "more" }, "state": "open", "draft": true },
            { "number": 40, "title": "", "head": { "ref": "seed" }, "state": "closed", "merged_at": "2026-01-01T00:00:00Z" },
        ] });
        let stack = read_stack(&raw).unwrap();
        assert_eq!((stack.number, stack.url.as_str(), stack.base.as_str()), (7, "https://github.com/o/r/stacks/7", "main"));
        assert_eq!(stack.layers.iter().map(|l| (l.number, l.state)).collect::<Vec<_>>(), vec![(42, State::Open), (41, State::Draft), (40, State::Merged)]);
        assert_eq!(read_stack(&json!({ "number": 8, "base": "dev", "url": "u", "pull_requests": [] })).unwrap().base, "dev");
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
        let r = row("github.com", &node, "me", &HashSet::new()).unwrap();
        assert_eq!((r.state, r.conflicting, r.review, r.checks, r.group), (State::Open, true, Some(Review::ChangesRequested), Some(Checks::Failing), Group::Reviewing));
        assert_eq!(r.labels, vec![Label { name: "bug".into(), color: "d73a4a".into() }]);
        let mine = row("github.com", &node, "someone", &HashSet::new()).unwrap();
        assert_eq!(mine.group, Group::Authored);
        let team = row("github.com", &json!({ "url": "https://github.com/o/r/pull/8", "number": 8, "state": "MERGED" }), "me", &["https://github.com/o/r/pull/8".to_owned()].into()).unwrap();
        assert_eq!((team.state, team.group, team.stack), (State::Merged, Group::Reviewing, None));
        let stacked = row("github.com", &json!({ "url": "u", "number": 9, "state": "OPEN", "stack": { "number": 3, "size": 2, "baseRefName": "main" }, "stackEntry": { "position": 2 } }), "me", &HashSet::new()).unwrap();
        assert_eq!(stacked.stack, Some(StackMembership { number: 3, position: 2, size: 2, base: "main".into() }));
    }
}
