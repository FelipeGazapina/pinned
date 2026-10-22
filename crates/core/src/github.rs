//! GitHub, whose token lets any machine of the account clone its private repositories.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const API: &str = "https://api.github.com";
/// Where a signed-in user creates the token pinned asks for, with the `repo` scope picked.
pub const TOKEN_PAGE: &str = "https://github.com/settings/tokens/new?scopes=repo&description=pinned";
/// Most repositories `repos` lists, newest push first.
const REPOS_LIMIT: usize = 500;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Account {
    pub login: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Repo {
    /// `owner/name`.
    pub full_name: String,
    pub clone_url: String,
    pub private: bool,
    #[serde(default)]
    pub description: String,
}

async fn get(token: &str, path: &str) -> Result<Value, String> {
    let response = reqwest::Client::new()
        .get(format!("{API}{path}"))
        .bearer_auth(token)
        .header("User-Agent", "pinned")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub unreachable: {e}"))?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|e| format!("GitHub sent a bad response: {e}"))?;
    match status.as_u16() {
        200..=299 => Ok(body),
        401 => Err("GitHub refused the token".into()),
        _ => Err(format!("GitHub answered {status}: {}", body["message"].as_str().unwrap_or_default())),
    }
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
    fn the_token_goes_in_a_basic_header_for_github_only() {
        assert_eq!(
            auth_config("ghp_1"),
            ("http.https://github.com/.extraheader", format!("AUTHORIZATION: basic {}", base64::engine::general_purpose::STANDARD.encode("x-access-token:ghp_1")))
        );
    }
}
