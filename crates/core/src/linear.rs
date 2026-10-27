use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const API: &str = "https://api.linear.app/graphql";
/// Where a signed-in user creates the personal API key pinned asks for.
pub const KEY_PAGE: &str = "https://linear.app/settings/account/security";

#[derive(Serialize, Deserialize, Clone, Debug)]
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

async fn query(key: &str, query: &str, variables: Value) -> Result<Value, String> {
    let response = reqwest::Client::new()
        .post(API)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
