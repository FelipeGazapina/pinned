//! PostHog error tracking, which an observability workflow reads. The HTTP JSON is parsed here and
//! never leaves the module. The key only goes to the host the user named: it is checked before any
//! request, and no request follows a redirect.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

/// PostHog's US cloud, the host `us` names.
pub const US: &str = "https://us.posthog.com";
/// PostHog's EU cloud, the host `eu` names.
pub const EU: &str = "https://eu.posthog.com";
/// The most error issues one check reads: the newest page only.
const PER_CHECK: usize = 100;
const CALL_WAIT: Duration = Duration::from_secs(30);
/// How much of an answer pinned reads. A self-hosted site is any host the user names, and the hub
/// asks it every minute, so a bigger answer is refused, never buffered. 100 issues fit many times over.
const BODY_MAX: usize = 2 * 1024 * 1024;
/// The longest issue id kept. PostHog's are UUIDs.
const ID_MAX: usize = 64;

/// What the Connectors card shows of the PostHog project pinned reads. The key is kept apart.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Account {
    /// The site requests go to, as `https://us.posthog.com`.
    pub host: String,
    /// The project id, as PostHog's project settings show it.
    pub project: String,
    pub project_name: String,
    /// The key's last characters, as `…a1b2`.
    pub key_hint: String,
}

/// An error-tracking issue: one error, however often it happened.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub occurrences: u64,
    pub first_seen: String,
    pub last_seen: String,
    pub status: String,
}

/// The site a host choice names: `us`, `eu`, or the root of a public `https://` site, without
/// a user, path, query or fragment. Refused before any request, so a key never goes elsewhere.
pub fn host(value: &str) -> Result<String, String> {
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "us" => return Ok(US.into()),
        "eu" => return Ok(EU.into()),
        _ => {}
    }
    let url = reqwest::Url::parse(value).map_err(|_| format!("PostHog host {value:?} is not us, eu or an https:// URL"))?;
    if url.scheme() != "https" {
        return Err(format!("PostHog host {value:?} must be an https:// URL"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("PostHog host must not hold a user name or password".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("PostHog host must not have a query or fragment".into());
    }
    if url.path() != "/" {
        return Err("PostHog host must be the site's address only, with no path".into());
    }
    let name = url.host_str().ok_or_else(|| format!("PostHog host {value:?} has no hostname"))?;
    if name.trim_start_matches('[').trim_end_matches(']').parse::<std::net::IpAddr>().is_ok() {
        return Err("PostHog host must be a hostname, not an IP address".into());
    }
    let bare = name.trim_end_matches('.').to_ascii_lowercase();
    if bare == "localhost" || bare.ends_with(".localhost") {
        return Err("PostHog host must be a public hostname, not localhost".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

/// A project id as PostHog writes it: digits only, so it is safe in a request path.
pub fn project(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("PostHog project id must be the number in the project's settings".into());
    }
    Ok(value.to_owned())
}

/// Where an issue opens in PostHog.
pub fn issue_url(host: &str, project: &str, id: &str) -> String {
    format!("{}/project/{project}/error_tracking/{id}", host.trim_end_matches('/'))
}

async fn call(method: reqwest::Method, host: &str, path: &str, key: &str, body: Option<&Value>) -> Result<(reqwest::StatusCode, Value), String> {
    let client = reqwest::Client::builder()
        .timeout(CALL_WAIT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("PostHog: {e}"))?;
    let mut request = client.request(method, format!("{host}{path}")).bearer_auth(key);
    if let Some(body) = body {
        request = request.json(body);
    }
    let mut response = request.send().await.map_err(|e| format!("PostHog at {host} is unreachable: {}", e.without_url()))?;
    let status = response.status();
    let mut bytes: Vec<u8> = vec![];
    while let Some(chunk) = response.chunk().await.map_err(|e| format!("PostHog sent a bad response: {}", e.without_url()))? {
        if bytes.len() + chunk.len() > BODY_MAX {
            return Err(format!("PostHog at {host} sent more than {} MB, which pinned does not read", BODY_MAX / (1024 * 1024)));
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, body))
}

/// Whether `id` is an issue id pinned keeps: letters, digits and `-`, as a UUID is, at most
/// `ID_MAX` long. It goes into the run's subject and the link a Linear ticket carries.
fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= ID_MAX && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn refused(status: reqwest::StatusCode, host: &str, project: &str) -> String {
    match status.as_u16() {
        401 => "PostHog refused the key: check the personal API key".into(),
        403 => format!("PostHog refused the key for project {project}: give it read access to the project and to queries"),
        404 => format!("PostHog has no project {project} at {host}"),
        300..=399 => format!("PostHog at {host} answered with a redirect, which pinned does not follow: use the site's own address"),
        code => format!("PostHog answered {code}"),
    }
}

/// Checks that `key` reads `project` on `host` before pinned keeps it.
pub async fn account(key: &str, project_id: &str, host_choice: &str) -> Result<Account, String> {
    let host = host(host_choice)?;
    let project = project(project_id)?;
    let key = key.trim();
    if key.is_empty() {
        return Err("PostHog needs a personal API key".into());
    }
    let (status, body) = call(reqwest::Method::GET, &host, &format!("/api/projects/{project}/"), key, None).await?;
    if !status.is_success() {
        return Err(refused(status, &host, &project));
    }
    // A site that answers anything with a 200 is not PostHog: its project has an id.
    if body["id"].is_null() {
        return Err(format!("{host} did not answer as PostHog does for project {project}: check the host"));
    }
    let project_name = body["name"].as_str().unwrap_or_default().to_owned();
    Ok(Account { host, project, project_name, key_hint: crate::account_copy::secret_tail(key) })
}

/// The project's active error issues first seen after `since`, newest first, at most `PER_CHECK`.
pub async fn new_issues(key: &str, account: &Account, since: DateTime<Utc>) -> Result<Vec<Issue>, String> {
    // The account was checked when it was kept; its host is checked again before the key goes there.
    let host = host(&account.host)?;
    let project = project(&account.project)?;
    let query = json!({
        "query": {
            "kind": "ErrorTrackingQuery",
            "dateRange": { "date_from": since.to_rfc3339() },
            "status": "active",
            "orderBy": "first_seen",
            "orderDirection": "DESC",
            "volumeResolution": 0,
            "withAggregations": true,
            "limit": PER_CHECK,
        }
    });
    let (status, body) = call(reqwest::Method::POST, &host, &format!("/api/projects/{project}/query/"), key.trim(), Some(&query)).await?;
    if !status.is_success() {
        return Err(refused(status, &host, &project));
    }
    issues_from(&body, since)
}

/// The active issues of an `ErrorTrackingQuery` answer first seen after `since`, at most `PER_CHECK`.
pub fn issues_from(body: &Value, since: DateTime<Utc>) -> Result<Vec<Issue>, String> {
    let results = body["results"].as_array().ok_or_else(|| "PostHog sent an error list that isn't one".to_owned())?;
    let text = |value: &Value| value.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let issues = results
        .iter()
        .filter_map(|item| {
            let id = text(&item["id"]).filter(|id| plain_id(id))?;
            let first_seen = text(&item["first_seen"])?;
            let after = DateTime::parse_from_rfc3339(&first_seen).ok()? > since;
            let status = text(&item["status"]).unwrap_or_default();
            (after && status == "active").then(|| Issue {
                title: text(&item["name"]).unwrap_or_else(|| "Unnamed error".into()),
                description: text(&item["description"]),
                occurrences: item["aggregations"]["occurrences"].as_f64().map_or(0, |n| n.max(0.0) as u64),
                last_seen: text(&item["last_seen"]).unwrap_or_else(|| first_seen.clone()),
                first_seen,
                status,
                id,
            })
        })
        .take(PER_CHECK)
        .collect();
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_takes_the_clouds_and_public_https_sites_only() {
        assert_eq!(host("us").as_deref(), Ok(US));
        assert_eq!(host(" EU ").as_deref(), Ok(EU));
        assert_eq!(host("https://posthog.example.com").as_deref(), Ok("https://posthog.example.com"));
        assert_eq!(host("https://posthog.example.com/").as_deref(), Ok("https://posthog.example.com"));
        for refused in [
            "http://x.example",
            "https://127.0.0.1",
            "https://[::1]",
            "https://10.0.0.8",
            "https://localhost",
            "https://app.localhost",
            "https://user:pw@x.example",
            "https://x.example?a=1",
            "https://x.example#top",
            "https://x.example/path",
            "x.example",
            "",
        ] {
            assert!(host(refused).is_err(), "{refused:?} is refused");
        }
    }

    #[test]
    fn a_project_id_is_digits() {
        assert_eq!(project(" 12345 ").as_deref(), Ok("12345"));
        for refused in ["", "abc", "12/../3", "1 2"] {
            assert!(project(refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn an_issue_id_is_letters_digits_and_dashes() {
        assert!(plain_id("0193abcd-ef01-7000-8000-0123456789ab"));
        let long = "a".repeat(65);
        for refused in ["", "a b", "x)\n[click](https://evil.example)", "../x", long.as_str()] {
            assert!(!plain_id(refused), "{refused:?}");
        }
    }

    #[test]
    fn an_issue_url_names_the_host_project_and_issue() {
        assert_eq!(issue_url("https://us.posthog.com", "12345", "0193-ab"), "https://us.posthog.com/project/12345/error_tracking/0193-ab");
    }

    /// The shape of an `ErrorTrackingQuery` answer from `POST /api/projects/<id>/query/`, trimmed.
    const FIXTURE: &str = r#"{
        "columns": ["id", "status", "name", "description", "first_seen", "last_seen", "aggregations"],
        "hasMore": false,
        "results": [
            {
                "id": "01933a7e-new",
                "status": "active",
                "name": "TypeError",
                "description": "Cannot read properties of undefined (reading 'id')",
                "first_seen": "2026-11-23T10:05:00Z",
                "last_seen": "2026-11-23T10:09:30Z",
                "aggregations": { "occurrences": 14, "sessions": 3, "users": 2, "volume_buckets": [] },
                "library": "web"
            },
            {
                "id": "01933a7e-bare",
                "status": "active",
                "name": null,
                "description": null,
                "first_seen": "2026-11-23T10:02:00.123000+00:00",
                "last_seen": "2026-11-23T10:02:00.123000+00:00"
            },
            {
                "id": "01933a7e-muted",
                "status": "suppressed",
                "name": "ResizeObserver loop limit exceeded",
                "description": "",
                "first_seen": "2026-11-23T10:03:00Z",
                "last_seen": "2026-11-23T10:04:00Z",
                "aggregations": { "occurrences": 900, "sessions": 80, "users": 70, "volume_buckets": [] }
            },
            {
                "id": "evil)\n[click](https://evil.example)",
                "status": "active",
                "name": "Injected",
                "first_seen": "2026-11-23T10:06:00Z",
                "last_seen": "2026-11-23T10:06:00Z"
            },
            {
                "id": "01933a7e-old",
                "status": "active",
                "name": "ReferenceError",
                "description": "x is not defined",
                "first_seen": "2026-11-20T08:00:00Z",
                "last_seen": "2026-11-23T10:01:00Z",
                "aggregations": { "occurrences": 40, "sessions": 9, "users": 5, "volume_buckets": [] }
            }
        ]
    }"#;

    #[test]
    fn issues_from_keeps_the_active_ones_first_seen_after_since() {
        let since = DateTime::parse_from_rfc3339("2026-11-23T10:00:00Z").unwrap().with_timezone(&Utc);
        let issues = issues_from(&serde_json::from_str(FIXTURE).unwrap(), since).unwrap();
        assert_eq!(
            issues,
            vec![
                Issue {
                    id: "01933a7e-new".into(),
                    title: "TypeError".into(),
                    description: Some("Cannot read properties of undefined (reading 'id')".into()),
                    occurrences: 14,
                    first_seen: "2026-11-23T10:05:00Z".into(),
                    last_seen: "2026-11-23T10:09:30Z".into(),
                    status: "active".into(),
                },
                Issue {
                    id: "01933a7e-bare".into(),
                    title: "Unnamed error".into(),
                    description: None,
                    occurrences: 0,
                    first_seen: "2026-11-23T10:02:00.123000+00:00".into(),
                    last_seen: "2026-11-23T10:02:00.123000+00:00".into(),
                    status: "active".into(),
                },
            ],
            "the suppressed issue, the one first seen before since and the one whose id is not plain are left out"
        );
        assert!(issues_from(&json!({ "error": "nope" }), since).is_err());
    }

    #[test]
    fn an_account_shows_the_key_hint_only() {
        let account = Account { host: US.into(), project: "1".into(), project_name: "Web".into(), key_hint: crate::account_copy::secret_tail("phx_secretkey1234") };
        let shown = serde_json::to_string(&account).unwrap();
        assert!(shown.contains("…1234") && !shown.contains("phx_secretkey1234"), "{shown}");
    }
}
