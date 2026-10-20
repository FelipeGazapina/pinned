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

#[derive(Deserialize, Debug)]
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

/// Issues carrying `label` that were created after `since` (RFC 3339).
pub async fn labeled_issues(key: &str, label: &str, since: &str) -> Result<Vec<Issue>, String> {
    let data = query(
        key,
        "query($label: String!, $since: DateTimeOrDuration!) {
           issues(first: 50, filter: { createdAt: { gt: $since }, labels: { some: { name: { eqIgnoreCase: $label } } } }) {
             nodes { id identifier title description url }
           }
         }",
        json!({ "label": label, "since": since }),
    )
    .await?;
    serde_json::from_value(data["issues"]["nodes"].clone()).map_err(|e| e.to_string())
}
