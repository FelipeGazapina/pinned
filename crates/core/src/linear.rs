use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const API: &str = "https://api.linear.app/graphql";
/// A debug build's stand-in for `API`, a loopback http(s) address only (`slack::loopback_root`).
pub const API_ENV: &str = "PINNED_LINEAR_API";
/// Where a signed-in user creates the personal API key pinned asks for.
pub const KEY_PAGE: &str = "https://linear.app/settings/account/security";

#[derive(Serialize, Deserialize, Clone, Debug, schemars::JsonSchema)]
pub struct Account {
    pub name: String,
    pub email: String,
    pub organization: String,
}

#[derive(Deserialize, Debug, PartialEq)]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
}

fn api() -> String {
    #[cfg(test)]
    if let Some(root) = API_ROOT.lock().unwrap().clone() {
        return root;
    }
    crate::slack::loopback_root(API_ENV, API)
}

/// Points Linear calls at `url` for a test. `None` restores the real API.
#[cfg(test)]
pub fn set_api(url: Option<String>) {
    *API_ROOT.lock().unwrap() = url;
}

#[cfg(test)]
static API_ROOT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

async fn query(key: &str, query: &str, variables: Value) -> Result<Value, String> {
    let response = reqwest::Client::new()
        .post(api())
        .timeout(crate::connectors::CALL_TIMEOUT)
        .header("Authorization", key)
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .map_err(|e| format!("Linear unreachable: {e}"))?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|e| format!("Linear sent a bad response: {e}"))?;
    if let Some(message) = body["errors"][0]["message"].as_str() {
        return Err(format!("Linear: {message}"));
    }
    if !status.is_success() {
        return Err(format!("Linear answered {status}"));
    }
    Ok(body["data"].clone())
}

pub async fn account(key: &str) -> Result<Account, String> {
    let data = query(key, "{ viewer { name email organization { name } } }", json!({})).await?;
    let viewer = &data["viewer"];
    let field = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    Ok(Account {
        name: field(&viewer["name"]),
        email: field(&viewer["email"]),
        organization: field(&viewer["organization"]["name"]),
    })
}

pub async fn label_names(key: &str) -> Result<Vec<String>, String> {
    let data = query(key, "{ issueLabels(first: 250) { nodes { name } } }", json!({})).await?;
    let mut names: Vec<String> = data["issueLabels"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n["name"].as_str().map(str::to_owned))
        .collect();
    names.sort_unstable_by_key(|n| n.to_lowercase());
    names.dedup();
    Ok(names)
}

/// Column names across every team, for suggesting a connector's columns.
pub async fn state_names(key: &str) -> Result<Vec<String>, String> {
    let data = query(key, "{ workflowStates(first: 250) { nodes { name } } }", json!({})).await?;
    let mut names: Vec<String> = data["workflowStates"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n["name"].as_str().map(str::to_owned))
        .collect();
    names.sort_unstable_by_key(|n| n.to_lowercase());
    names.dedup_by_key(|n| n.to_lowercase());
    Ok(names)
}

/// Moves the issue to its team's column called `column`, ignoring case.
pub async fn move_issue(key: &str, issue: &str, column: &str) -> Result<(), String> {
    let data = query(
        key,
        "query($id: String!) { issue(id: $id) { identifier team { name states { nodes { id name } } } } }",
        json!({ "id": issue }),
    )
    .await?;
    let identifier = data["issue"]["identifier"].as_str().unwrap_or(issue).to_owned();
    let team = &data["issue"]["team"];
    let state = team["states"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(column)))
        .and_then(|s| s["id"].as_str())
        .ok_or_else(|| format!("team {} has no column named {column:?}", team["name"].as_str().unwrap_or("?")))?;
    let updated = query(
        key,
        "mutation($id: String!, $state: String!) { issueUpdate(id: $id, input: { stateId: $state }) { success } }",
        json!({ "id": issue, "state": state }),
    )
    .await?;
    match updated["issueUpdate"]["success"].as_bool() {
        Some(true) => Ok(()),
        _ => Err(format!("Linear refused to move {identifier} to {column:?}")),
    }
}

/// Team names, for suggesting the team a Slack connector files issues in.
pub async fn team_names(key: &str) -> Result<Vec<String>, String> {
    let data = query(key, "{ teams(first: 250) { nodes { name } } }", json!({})).await?;
    let mut names: Vec<String> = nodes(&data["teams"]).filter_map(|n| n["name"].as_str().map(str::to_owned)).collect();
    names.sort_unstable_by_key(|n| n.to_lowercase());
    Ok(names)
}

fn nodes(connection: &Value) -> impl Iterator<Item = &Value> {
    connection["nodes"].as_array().into_iter().flatten()
}

/// The ids of the team called `team`, by name or key, and of the label called `label`, which
/// may belong to the workspace or to that team. Both ignore case; an empty label resolves to none.
fn resolve(data: &Value, team: &str, label: &str) -> Result<(String, Option<String>), String> {
    let field = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    let found = nodes(&data["teams"])
        .find(|t| [&t["name"], &t["key"]].iter().any(|v| v.as_str().is_some_and(|n| n.eq_ignore_ascii_case(team.trim()))))
        .ok_or_else(|| {
            let teams: Vec<String> = nodes(&data["teams"]).map(|t| format!("{} ({})", field(t, "name"), field(t, "key"))).collect();
            format!("Linear has no team named {team:?}; its teams are {}", teams.join(", "))
        })?;
    let team_id = field(found, "id");
    if label.trim().is_empty() {
        return Ok((team_id, None));
    }
    let label_id = nodes(&data["issueLabels"])
        .filter(|l| l["team"].is_null() || l["team"]["id"].as_str() == Some(&team_id))
        .find(|l| l["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(label.trim())))
        .map(|l| field(l, "id"))
        .ok_or_else(|| format!("team {} has no label named {label:?}", field(found, "name")))?;
    Ok((team_id, Some(label_id)))
}

/// The id of the user an issue pinned files is assigned to: `who`, by name, display name or email
/// ignoring case, or the owner of the API key when `who` is empty, since pinned files on their behalf.
fn assignee(data: &Value, who: &str) -> Result<String, String> {
    let who = who.trim();
    let id = if who.is_empty() {
        data["viewer"]["id"].as_str()
    } else {
        nodes(&data["users"])
            .find(|u| ["name", "displayName", "email"].iter().any(|k| u[k].as_str().is_some_and(|v| v.eq_ignore_ascii_case(who))))
            .and_then(|u| u["id"].as_str())
    };
    id.map(str::to_owned).ok_or_else(|| if who.is_empty() { "Linear did not say who owns the API key".into() } else { format!("Linear has no user named {who:?}") })
}

fn create_input(team: &str, assignee: &str, title: &str, description: &str, label: Option<&str>) -> Value {
    let mut input = json!({ "teamId": team, "assigneeId": assignee, "title": title, "description": description });
    if let Some(label) = label {
        input["labelIds"] = json!([label]);
    }
    input
}

fn created(data: &Value) -> Result<Issue, String> {
    let result = &data["issueCreate"];
    if result["success"].as_bool() != Some(true) {
        return Err("Linear refused to create the issue".into());
    }
    serde_json::from_value(result["issue"].clone()).map_err(|e| format!("Linear sent a bad issue: {e}"))
}

/// Creates an issue in `team`, a team name or key, with the label called `label` unless it is empty,
/// assigned to the user `assignee` names, or to the owner of the API key when it is empty.
pub async fn create_issue(key: &str, team: &str, title: &str, description: &str, label: &str, assignee: &str) -> Result<Issue, String> {
    let data = query(
        key,
        "{ viewer { id } users(first: 250) { nodes { id name displayName email } } \
           teams(first: 250) { nodes { id key name } } issueLabels(first: 250) { nodes { id name team { id } } } }",
        json!({}),
    )
    .await?;
    let (team, label) = resolve(&data, team, label)?;
    let assignee = self::assignee(&data, assignee)?;
    let data = query(
        key,
        "mutation($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { id identifier title description url } } }",
        json!({ "input": create_input(&team, &assignee, title, description, label.as_deref()) }),
    )
    .await?;
    created(&data)
}

/// The issue `id` names, either Linear's id or an identifier such as ENG-123.
pub async fn issue(key: &str, id: &str) -> Result<Issue, String> {
    let data = query(key, "query($id: String!) { issue(id: $id) { id identifier title description url } }", json!({ "id": id })).await?;
    if data["issue"].is_null() {
        return Err(format!("Linear has no issue {id}"));
    }
    serde_json::from_value(data["issue"].clone()).map_err(|e| format!("Linear sent a bad issue: {e}"))
}

/// The issues in `data` that were created after `since`, or had `label` added after it.
fn labeled(data: &Value, label: &str, since: &str) -> Result<Vec<Issue>, String> {
    let since = DateTime::parse_from_rfc3339(since).map_err(|e| format!("bad time {since:?}: {e}"))?;
    let after = |v: &Value| v.as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).is_some_and(|t| t > since);
    let adds_label = |h: &Value| h["addedLabels"].as_array().into_iter().flatten().any(|l| l["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(label)));
    nodes(&data["issues"])
        .filter(|i| after(&i["createdAt"]) || nodes(&i["history"]).any(|h| after(&h["createdAt"]) && adds_label(h)))
        .map(|i| serde_json::from_value(i.clone()).map_err(|e| format!("Linear sent a bad issue: {e}")))
        .collect()
}

/// Issues carrying `label` that were created after `since` (RFC 3339), or that had `label` added
/// after it. Adding a label bumps `updatedAt`, so filtering on that finds both kinds.
pub async fn labeled_issues(key: &str, label: &str, since: &str) -> Result<Vec<Issue>, String> {
    let data = query(
        key,
        "query($label: String!, $since: DateTimeOrDuration!) {
           issues(first: 50, orderBy: updatedAt, filter: { updatedAt: { gt: $since }, labels: { some: { name: { eqIgnoreCase: $label } } } }) {
             nodes { id identifier title description url createdAt history(first: 50) { nodes { createdAt addedLabels { name } } } }
           }
         }",
        json!({ "label": label, "since": since }),
    )
    .await?;
    labeled(&data, label, since)
}

/// A person in Linear: who made a change, or who an issue is assigned to.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub id: String,
    pub name: String,
}

/// What a `Change` changed: the issue's status, or its assignee.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChangeKind {
    Status,
    Assignee,
}

/// One change to an issue, read from its history: a status move, or an assignee change (an unassign too).
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub issue_id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    /// Linear's id for the history entry. An entry that changes both status and assignee gives two changes with this id.
    pub history_id: String,
    /// When the change happened, as Linear sent it (RFC 3339).
    pub at: String,
    pub actor: Person,
    pub kind: ChangeKind,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub from_assignee: Option<Person>,
    pub to_assignee: Option<Person>,
    pub team_key: String,
    pub team_name: String,
    pub project: Option<String>,
}

/// Which issues a read covers: a team and a project, each by name or key. Empty means any.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scope {
    pub team: String,
    pub project: String,
}

/// The changes one read found. `oldest` is set only when the read stopped at its page cap: the caller then
/// moves its watermark to `oldest`, the oldest `updatedAt` read, and not further.
#[derive(Debug, PartialEq)]
pub struct Changes {
    pub changes: Vec<Change>,
    pub oldest: Option<String>,
}

/// The changes to issues in `scope` made after `since` (RFC 3339), read up to four pages of 50 issues.
pub async fn changes(_key: &str, _since: &str, _scope: &Scope) -> Result<Changes, String> {
    Ok(Changes { changes: Vec::new(), oldest: None })
}

/// The changes in the issues of `data` made after `since`.
fn changes_in(_data: &Value, _since: &str) -> Result<Vec<Change>, String> {
    Ok(Vec::new())
}

/// The id of the user the API key `key` belongs to. Read once per key, then kept in memory.
pub async fn viewer_id(_key: &str) -> Result<String, String> {
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn workspace() -> Value {
        json!({
            "teams": { "nodes": [
                { "id": "t-eng", "key": "ENG", "name": "Engineering" },
                { "id": "t-des", "key": "DES", "name": "Design" },
            ] },
            "issueLabels": { "nodes": [
                { "id": "l-bug-des", "name": "Bug", "team": { "id": "t-des" } },
                { "id": "l-bug", "name": "Bug", "team": null },
                { "id": "l-slack", "name": "from-slack", "team": { "id": "t-eng" } },
            ] },
        })
    }

    #[test]
    fn teams_resolve_by_name_or_key_ignoring_case() {
        assert_eq!(resolve(&workspace(), "engineering", ""), Ok(("t-eng".into(), None)));
        assert_eq!(resolve(&workspace(), " des ", ""), Ok(("t-des".into(), None)));
        assert_eq!(
            resolve(&workspace(), "Platform", "bug"),
            Err("Linear has no team named \"Platform\"; its teams are Engineering (ENG), Design (DES)".into())
        );
    }

    #[test]
    fn labels_resolve_within_the_workspace_or_the_team() {
        assert_eq!(resolve(&workspace(), "ENG", "bug"), Ok(("t-eng".into(), Some("l-bug".into()))));
        assert_eq!(resolve(&workspace(), "ENG", "From-Slack"), Ok(("t-eng".into(), Some("l-slack".into()))));
        assert_eq!(resolve(&workspace(), "Design", "from-slack"), Err("team Design has no label named \"from-slack\"".into()));
    }

    #[test]
    fn create_sends_the_label_only_when_there_is_one() {
        assert_eq!(
            create_input("t-eng", "u-me", "Export 500s", "From Slack", Some("l-bug")),
            json!({ "teamId": "t-eng", "assigneeId": "u-me", "title": "Export 500s", "description": "From Slack", "labelIds": ["l-bug"] })
        );
        assert_eq!(
            create_input("t-eng", "u-me", "Export 500s", "", None),
            json!({ "teamId": "t-eng", "assigneeId": "u-me", "title": "Export 500s", "description": "" })
        );
    }

    #[test]
    fn an_issue_goes_to_the_key_owner_unless_someone_is_named() {
        let data = json!({
            "viewer": { "id": "u-me" },
            "users": { "nodes": [
                { "id": "u-me", "name": "Felipe Gazapina", "displayName": "felipe", "email": "felipe@example.com" },
                { "id": "u-ana", "name": "Ana Souza", "displayName": "ana", "email": "ana@example.com" },
            ] },
        });
        assert_eq!(assignee(&data, ""), Ok("u-me".into()));
        assert_eq!(assignee(&data, "  "), Ok("u-me".into()));
        assert_eq!(assignee(&data, "Ana"), Ok("u-ana".into()));
        assert_eq!(assignee(&data, "ana souza"), Ok("u-ana".into()));
        assert_eq!(assignee(&data, "ANA@example.com"), Ok("u-ana".into()));
        assert_eq!(assignee(&data, "bruno"), Err("Linear has no user named \"bruno\"".into()));
        assert_eq!(assignee(&json!({}), ""), Err("Linear did not say who owns the API key".into()));
    }

    #[test]
    fn a_created_issue_is_read_from_the_mutation() {
        let data = json!({ "issueCreate": { "success": true, "issue": {
            "id": "uuid", "identifier": "ENG-12", "title": "Export 500s", "description": "From Slack", "url": "https://linear.app/x/issue/ENG-12",
        } } });
        assert_eq!(
            created(&data),
            Ok(Issue {
                id: "uuid".into(),
                identifier: "ENG-12".into(),
                title: "Export 500s".into(),
                description: Some("From Slack".into()),
                url: "https://linear.app/x/issue/ENG-12".into(),
            })
        );
        assert_eq!(created(&json!({ "issueCreate": { "success": false } })), Err("Linear refused to create the issue".into()));
    }

    fn issue_node(identifier: &str, created_at: &str, history: Value) -> Value {
        json!({
            "id": identifier, "identifier": identifier, "title": "t", "description": null, "url": "u",
            "createdAt": created_at, "history": { "nodes": history },
        })
    }

    #[test]
    fn an_issue_counts_when_created_or_given_the_label_after_since() {
        let data = json!({ "issues": { "nodes": [
            issue_node("NEW-1", "2026-10-22T10:00:00.000Z", json!([
                { "createdAt": "2026-10-22T10:00:00.000Z", "addedLabels": [{ "name": "AGENTIC" }] },
            ])),
            issue_node("OLD-1", "2026-10-08T10:00:00.000Z", json!([
                { "createdAt": "2026-10-22T20:42:30.126Z", "addedLabels": [{ "name": "agentic" }] },
                { "createdAt": "2026-10-08T10:00:00.000Z", "addedLabels": null },
            ])),
            issue_node("OLD-2", "2026-10-08T10:00:00.000Z", json!([
                { "createdAt": "2026-10-23T09:00:00.000Z", "addedLabels": null },
                { "createdAt": "2026-10-09T10:00:00.000Z", "addedLabels": [{ "name": "AGENTIC" }] },
            ])),
            issue_node("OLD-3", "2026-10-08T10:00:00.000Z", json!([
                { "createdAt": "2026-10-23T09:00:00.000Z", "addedLabels": [{ "name": "Bug" }] },
                { "createdAt": "2026-10-09T10:00:00.000Z", "addedLabels": [{ "name": "AGENTIC" }] },
            ])),
        ] } });
        let found = labeled(&data, "AGENTIC", "2026-10-21T17:52:43.772566+00:00").unwrap();
        assert_eq!(found.iter().map(|i| i.identifier.as_str()).collect::<Vec<_>>(), ["NEW-1", "OLD-1"]);
        assert_eq!(
            found[1],
            Issue { id: "OLD-1".into(), identifier: "OLD-1".into(), title: "t".into(), description: None, url: "u".into() }
        );
    }

    #[test]
    fn a_bad_since_is_an_error() {
        assert_eq!(
            labeled(&json!({ "issues": { "nodes": [] } }), "AGENTIC", "yesterday"),
            Err("bad time \"yesterday\": premature end of input".into())
        );
    }

    const SINCE: &str = "2026-11-09T18:00:00.000Z";

    fn person(id: &str, name: &str) -> Person {
        Person { id: id.into(), name: name.into() }
    }

    /// ENG-1 with the given history entries, oldest first.
    fn issue_with(history: Value) -> Value {
        json!({ "issues": { "nodes": [{
            "id": "i-1", "identifier": "ENG-1", "title": "Export 500s", "description": null,
            "url": "https://linear.app/x/issue/ENG-1", "updatedAt": "2026-11-10T13:00:00.000Z",
            "team": { "key": "ENG", "name": "Engineering" }, "project": { "name": "Exports" },
            "history": { "nodes": history },
        }] } })
    }

    /// The change to ENG-1 that `history_id` is, with no from or to fields set.
    fn change(history_id: &str, at: &str, actor: Person, kind: ChangeKind) -> Change {
        Change {
            issue_id: "i-1".into(),
            identifier: "ENG-1".into(),
            title: "Export 500s".into(),
            description: None,
            url: "https://linear.app/x/issue/ENG-1".into(),
            history_id: history_id.into(),
            at: at.into(),
            actor,
            kind,
            from_state: None,
            to_state: None,
            from_assignee: None,
            to_assignee: None,
            team_key: "ENG".into(),
            team_name: "Engineering".into(),
            project: Some("Exports".into()),
        }
    }

    /// A history entry that changes nothing but a label: all four change fields are null, as Linear sends them.
    fn label_only() -> Value {
        json!({ "id": "h-label", "createdAt": "2026-11-10T13:00:00.000Z", "actor": { "id": "u-ana", "name": "Ana Souza" },
            "fromState": null, "toState": null, "fromAssignee": null, "toAssignee": null })
    }

    #[test]
    fn a_status_move_and_assignee_changes_after_since_are_reported_and_nothing_else() {
        let found = changes_in(
            &issue_with(json!([
                { "id": "h-state", "createdAt": "2026-11-10T10:00:00.000Z", "actor": { "id": "u-ana", "name": "Ana Souza" },
                  "fromState": { "name": "Todo" }, "toState": { "name": "Ready to Dev" }, "fromAssignee": null, "toAssignee": null },
                { "id": "h-assign", "createdAt": "2026-11-10T11:00:00.000Z", "actor": { "id": "u-ana", "name": "Ana Souza" },
                  "fromState": null, "toState": null, "fromAssignee": null, "toAssignee": { "id": "u-bruno", "name": "Bruno Lima" } },
                { "id": "h-unassign", "createdAt": "2026-11-10T12:00:00.000Z", "actor": { "id": "u-bruno", "name": "Bruno Lima" },
                  "fromState": null, "toState": null, "fromAssignee": { "id": "u-bruno", "name": "Bruno Lima" }, "toAssignee": null },
                label_only(),
                { "id": "h-old", "createdAt": "2026-11-09T10:00:00.000Z", "actor": { "id": "u-ana", "name": "Ana Souza" },
                  "fromState": { "name": "Backlog" }, "toState": { "name": "Todo" }, "fromAssignee": null, "toAssignee": null },
            ])),
            SINCE,
        );
        assert_eq!(
            found,
            Ok(vec![
                Change {
                    from_state: Some("Todo".into()),
                    to_state: Some("Ready to Dev".into()),
                    ..change("h-state", "2026-11-10T10:00:00.000Z", person("u-ana", "Ana Souza"), ChangeKind::Status)
                },
                Change {
                    to_assignee: Some(person("u-bruno", "Bruno Lima")),
                    ..change("h-assign", "2026-11-10T11:00:00.000Z", person("u-ana", "Ana Souza"), ChangeKind::Assignee)
                },
                Change {
                    from_assignee: Some(person("u-bruno", "Bruno Lima")),
                    ..change("h-unassign", "2026-11-10T12:00:00.000Z", person("u-bruno", "Bruno Lima"), ChangeKind::Assignee)
                },
            ])
        );
    }

    #[test]
    fn an_entry_that_moves_status_and_changes_assignee_gives_two_changes() {
        let found = changes_in(
            &issue_with(json!([
                { "id": "h-both", "createdAt": "2026-11-10T10:00:00.000Z", "actor": { "id": "u-bruno", "name": "Bruno Lima" },
                  "fromState": { "name": "Ready to Dev" }, "toState": { "name": "In Progress" },
                  "fromAssignee": null, "toAssignee": { "id": "u-ana", "name": "Ana Souza" } },
            ])),
            SINCE,
        );
        assert_eq!(
            found,
            Ok(vec![
                Change {
                    from_state: Some("Ready to Dev".into()),
                    to_state: Some("In Progress".into()),
                    ..change("h-both", "2026-11-10T10:00:00.000Z", person("u-bruno", "Bruno Lima"), ChangeKind::Status)
                },
                Change {
                    to_assignee: Some(person("u-ana", "Ana Souza")),
                    ..change("h-both", "2026-11-10T10:00:00.000Z", person("u-bruno", "Bruno Lima"), ChangeKind::Assignee)
                },
            ])
        );
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
    }

    /// Points Linear back at the real API when a test ends.
    struct RestoreLinear;

    impl Drop for RestoreLinear {
        fn drop(&mut self) {
            set_api(None);
        }
    }

    /// The body of the one HTTP request on `stream`, read up to its Content-Length.
    fn read_body(stream: &mut std::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        while let Ok(n) = std::io::Read::read(stream, &mut chunk) {
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
            let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
            let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
            if buf.len() >= end + 4 + length {
                return String::from_utf8_lossy(&buf[end + 4..]).into_owned();
            }
        }
        String::new()
    }

    /// A loopback Linear for one test. `answer` gets each request's JSON body and returns the status and body
    /// to send back. Every request body is kept in order, for the test to read.
    fn stand_in(answer: impl Fn(&Value) -> (u16, String) + Send + 'static) -> (String, Arc<Mutex<Vec<Value>>>) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let root = format!("http://{}/graphql", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let body: Value = serde_json::from_str(&read_body(&mut stream)).unwrap_or(Value::Null);
                kept.lock().unwrap().push(body.clone());
                let (status, reply) = answer(&body);
                let head = format!("HTTP/1.1 {status} stand-in\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len());
                let _ = std::io::Write::write_all(&mut stream, format!("{head}{reply}").as_bytes());
            }
        });
        (root, seen)
    }

    /// Page `n` of a read whose issues are updated on day 20 minus `n`. Page `n` holds the issue ENG-n and
    /// has a next page while `n` is before `last`.
    fn page_of(request: &Value, last: u32) -> (u16, String) {
        let page = request["variables"]["after"].as_str().map_or(1, |cursor| cursor[1..].parse::<u32>().unwrap() + 1);
        let issue = json!({
            "id": format!("i-{page}"), "identifier": format!("ENG-{page}"), "title": "t", "description": null, "url": "u",
            "updatedAt": format!("2026-11-{:02}T10:00:00.000Z", 20 - page), "team": { "key": "ENG", "name": "Engineering" },
            "project": null, "history": { "nodes": [] },
        });
        let info = json!({ "hasNextPage": page < last, "endCursor": format!("c{page}") });
        (200, json!({ "data": { "issues": { "nodes": [issue], "pageInfo": info } } }).to_string())
    }

    #[test]
    fn a_read_stops_at_four_pages_and_names_the_oldest_update_it_read() {
        let _api = crate::slack::lock_api();
        let _restore = RestoreLinear;
        let (root, seen) = stand_in(|request| page_of(request, u32::MAX));
        set_api(Some(root));
        let read = block_on(changes("lin-cap", SINCE, &Scope::default())).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 4, "the read asks for four pages and no more");
        assert_eq!(read.oldest.as_deref(), Some("2026-11-16T10:00:00.000Z"), "the oldest updatedAt read is the watermark");
        assert_eq!(read.changes, Vec::new());
    }

    #[test]
    fn a_read_that_ends_before_the_cap_names_no_watermark() {
        let _api = crate::slack::lock_api();
        let _restore = RestoreLinear;
        let (root, seen) = stand_in(|request| page_of(request, 2));
        set_api(Some(root));
        let read = block_on(changes("lin-short", SINCE, &Scope::default())).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2, "the read stops when Linear says there is no next page");
        assert_eq!(read.oldest, None);
    }

    #[test]
    fn a_429_a_500_and_a_graphql_error_are_named_by_class_only() {
        let _api = crate::slack::lock_api();
        let _restore = RestoreLinear;
        let (root, _) = stand_in(|_| (429, json!({ "errors": [{ "message": "slow down for Export 500s" } ] }).to_string()));
        set_api(Some(root));
        assert_eq!(block_on(changes("lin-429", SINCE, &Scope::default())), Err("Linear answered 429".into()));

        let (root, _) = stand_in(|request| if request["variables"]["after"].is_null() { page_of(request, u32::MAX) } else { (500, "upstream text".into()) });
        set_api(Some(root));
        assert_eq!(block_on(changes("lin-500", SINCE, &Scope::default())), Err("Linear answered 500".into()), "a failed later page is an error, not a partial read");

        let (root, _) = stand_in(|_| (200, json!({ "errors": [{ "message": "invalid filter for Ana Souza" }] }).to_string()));
        set_api(Some(root));
        assert_eq!(block_on(changes("lin-gql", SINCE, &Scope::default())), Err("Linear answered with a GraphQL error".into()));
    }

    #[test]
    fn the_scope_and_the_watermark_go_into_the_filter() {
        let _api = crate::slack::lock_api();
        let _restore = RestoreLinear;
        let (root, seen) = stand_in(|request| page_of(request, 1));
        set_api(Some(root));
        let scope = Scope { team: " ENG ".into(), project: "Exports".into() };
        block_on(changes("lin-scope", SINCE, &scope)).unwrap();
        block_on(changes("lin-scope", SINCE, &Scope::default())).unwrap();
        let sent = seen.lock().unwrap().clone();
        assert_eq!(
            sent[0]["variables"]["filter"],
            json!({
                "updatedAt": { "gt": SINCE },
                "team": { "or": [{ "key": { "eqIgnoreCase": "ENG" } }, { "name": { "eqIgnoreCase": "ENG" } }] },
                "project": { "name": { "eqIgnoreCase": "Exports" } },
            })
        );
        assert_eq!(sent[1]["variables"]["filter"], json!({ "updatedAt": { "gt": SINCE } }), "an empty scope filters by time alone");
    }

    #[test]
    fn a_bad_since_is_an_error_before_any_request() {
        let _api = crate::slack::lock_api();
        let _restore = RestoreLinear;
        let (root, seen) = stand_in(|request| page_of(request, 1));
        set_api(Some(root));
        assert_eq!(
            block_on(changes("lin-since", "yesterday", &Scope::default())),
            Err("bad time \"yesterday\": premature end of input".into())
        );
        assert_eq!(seen.lock().unwrap().len(), 0);
    }

    #[test]
    fn the_viewer_is_read_once_per_key() {
        let _api = crate::slack::lock_api();
        let _restore = RestoreLinear;
        let (root, seen) = stand_in(|_| (200, json!({ "data": { "viewer": { "id": "u-viewer" } } }).to_string()));
        set_api(Some(root));
        assert_eq!(block_on(viewer_id("lin-viewer-once")), Ok("u-viewer".into()));
        assert_eq!(block_on(viewer_id("lin-viewer-once")), Ok("u-viewer".into()));
        assert_eq!(seen.lock().unwrap().len(), 1, "a second call with the same key is answered from memory");
        assert_eq!(block_on(viewer_id("lin-viewer-other")), Ok("u-viewer".into()));
        assert_eq!(seen.lock().unwrap().len(), 2, "a new key is read");
    }
}
