//! Who is on a team: making one, listing them, invites, members, leaving.

use super::{db, Scope};
use crate::hub::{Account, Hub};
use pinned_core::teams::{CredentialState, Member, Team, TeamInvite};
use std::sync::Arc;
use uuid::Uuid;

/// The longest team name, in characters.
pub const NAME_CHARS: usize = 80;

/// A team name trimmed, or why it is not one.
pub fn team_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > NAME_CHARS || name.contains('\0') {
        return Err(format!("a team needs a name of 1 to {NAME_CHARS} characters"));
    }
    Ok(name)
}

/// Which of the team account's keys are set: Upstash, a Claude token for boxes (the Upstash one
/// or a named one), GitHub, Slack. Never a value.
pub async fn credential(hub: &Arc<Hub>, team: &Account) -> Result<CredentialState, String> {
    let settings = hub.connectors(team).await?.get_settings();
    Ok(CredentialState {
        upstash: settings.upstash.is_some(),
        claude: settings.upstash.as_ref().is_some_and(|u| !u.claude_hint.is_empty()) || !settings.box_claude_tokens.is_empty(),
        github: settings.github.is_some(),
        slack: settings.slack.is_some() || !settings.slack_bots.is_empty(),
    })
}

/// Every team `person` is a member of, with its role and the team's keys' presence.
pub async fn list(hub: &Arc<Hub>, person: Uuid) -> Result<Vec<Team>, String> {
    let mut teams = Vec::new();
    for row in db::teams_of(&hub.db, person).await? {
        let account = hub.account(row.id).await?;
        let credential = credential(hub, &account).await?;
        teams.push(Team { id: row.id.to_string(), name: row.name, owner: row.owner.to_string(), role: row.role, credential });
    }
    Ok(teams)
}

/// Makes a team named `name` with `person` its owner and only member. The team is an account of
/// kind 'team' with no login: `db::create_team`.
pub async fn create(hub: &Arc<Hub>, person: Uuid, name: &str) -> Result<Team, String> {
    let name = team_name(name)?;
    let id = db::create_team(&hub.db, person, name).await?;
    eprintln!("pinned-server: account {person} made team {id}");
    Ok(Team { id: id.to_string(), name: name.to_owned(), owner: person.to_string(), role: pinned_core::teams::Role::Owner, credential: CredentialState::default() })
}

/// Joins the team of invite `code` as a member, once: `db::take_invite` with the code's sha256
/// (`crate::db::hash`). Unknown, used and revoked codes all answer the same error. A member of
/// the team already keeps its role. Answers the team as `list` shows it.
pub async fn join(_hub: &Arc<Hub>, _person: Uuid, _code: &str) -> Result<Team, String> {
    Err("not built yet".into())
}

/// Owner only (`Scope::owner_only`). Renames the team, `team_name` rules.
pub async fn rename(_hub: &Arc<Hub>, _scope: &Scope, _name: &str) -> Result<(), String> {
    Err("not built yet".into())
}

/// The members with their emails, roles and group handles, the owner first. Any member reads it.
pub async fn members(_hub: &Arc<Hub>, _scope: &Scope) -> Result<Vec<Member>, String> {
    Err("not built yet".into())
}

/// Owner only. A new single-use invite code (`crate::db::invite_code`); the hub keeps only its
/// sha256 and the hint (its first group), and shows the code this once.
pub async fn invite(_hub: &Arc<Hub>, _scope: &Scope) -> Result<TeamInvite, String> {
    Err("not built yet".into())
}

/// Owner only. Revokes the unused invites whose hint is `hint`; an error when there was none.
pub async fn revoke_invite(_hub: &Arc<Hub>, _scope: &Scope, _hint: &str) -> Result<(), String> {
    Err("not built yet".into())
}

/// Owner only. Removes member `account` (never the owner). Its groups lose it by cascade; the
/// projects it attached go with it (`team_projects.member` cascades); its runs keep going.
pub async fn remove_member(_hub: &Arc<Hub>, _scope: &Scope, _account: &str) -> Result<(), String> {
    Err("not built yet".into())
}

/// The member leaves the team. The owner cannot leave: it removes the others first, or the team.
pub async fn leave(_hub: &Arc<Hub>, _scope: &Scope) -> Result<(), String> {
    Err("not built yet".into())
}
