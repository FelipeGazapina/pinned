//! Who is on a team: making one, listing them, invites, members, leaving.

use super::{db, Scope};
use crate::db::{invite_code, invite_hash};
use crate::hub::{Account, Hub};
use pinned_core::teams::{CredentialState, Member, Role, Team, TeamInvite};
use std::sync::Arc;
use uuid::Uuid;

/// The longest team name, in characters.
pub const NAME_CHARS: usize = 80;

/// The one answer to an invite code that is unknown, used or revoked, so a stranger learns nothing.
const NO_INVITE: &str = "that invite code is unknown, used or revoked";

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

/// A team as `member` sees it, from its `teams_of` row.
async fn view(hub: &Arc<Hub>, row: db::TeamRow) -> Result<Team, String> {
    let account = hub.account(row.id).await?;
    let credential = credential(hub, &account).await?;
    Ok(Team { id: row.id.to_string(), name: row.name, owner: row.owner.to_string(), role: row.role, credential })
}

/// Every team `person` is a member of, with its role and the team's keys' presence.
pub async fn list(hub: &Arc<Hub>, person: Uuid) -> Result<Vec<Team>, String> {
    let mut teams = Vec::new();
    for row in db::teams_of(&hub.db, person).await? {
        teams.push(view(hub, row).await?);
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
/// (`crate::db::invite_hash`, which ignores case, spaces and dashes). Unknown, used and revoked codes all
/// answer the same error. A member of the team already keeps its role. Answers the team as `list` shows it.
pub async fn join(hub: &Arc<Hub>, person: Uuid, code: &str) -> Result<Team, String> {
    let team = db::take_invite(&hub.db, &invite_hash(code), person).await?.ok_or(NO_INVITE)?;
    let row = db::teams_of(&hub.db, person).await?.into_iter().find(|row| row.id == team).ok_or(NO_INVITE)?;
    eprintln!("pinned-server: account {person} joined team {team}");
    view(hub, row).await
}

/// Owner only (`Scope::owner_only`). Renames the team, `team_name` rules.
pub async fn rename(hub: &Arc<Hub>, scope: &Scope, name: &str) -> Result<(), String> {
    scope.owner_only()?;
    let name = team_name(name)?;
    if !db::rename_team(&hub.db, scope.team.id, name).await? {
        return Err("the team does not exist".into());
    }
    Ok(())
}

/// The members with their emails, roles and group handles, the owner first. Any member reads it.
pub async fn members(hub: &Arc<Hub>, scope: &Scope) -> Result<Vec<Member>, String> {
    let groups = db::member_groups(&hub.db, scope.team.id).await?;
    let rows = db::members(&hub.db, scope.team.id).await?;
    Ok(rows
        .into_iter()
        .map(|row| Member {
            account: row.account.to_string(),
            groups: groups.iter().filter(|(member, _)| *member == row.account).map(|(_, handle)| handle.clone()).collect(),
            email: row.email,
            role: row.role,
        })
        .collect())
}

/// Owner only. A new single-use invite code (`crate::db::invite_code`); the hub keeps only its
/// sha256 and the hint (its first group), and shows the code this once.
pub async fn invite(hub: &Arc<Hub>, scope: &Scope) -> Result<TeamInvite, String> {
    scope.owner_only()?;
    let code = invite_code();
    let hint = code[..4].to_owned();
    db::create_invite(&hub.db, scope.team.id, scope.member, &invite_hash(&code), &hint).await?;
    eprintln!("pinned-server: account {} made an invite to team {}", scope.member, scope.team.id);
    Ok(TeamInvite { code, hint })
}

/// Owner only. Revokes the unused invites whose hint is `hint`; an error when there was none.
pub async fn revoke_invite(hub: &Arc<Hub>, scope: &Scope, hint: &str) -> Result<(), String> {
    scope.owner_only()?;
    let hint = hint.trim().to_ascii_uppercase();
    match db::revoke_invite(&hub.db, scope.team.id, &hint).await? {
        0 => Err("there is no unused invite with that hint".into()),
        _ => Ok(()),
    }
}

/// Owner only. Removes member `account` (never the owner). Its groups lose it by cascade; the
/// projects it attached go with it (`db::remove_member`); its runs keep going.
pub async fn remove_member(hub: &Arc<Hub>, scope: &Scope, account: &str) -> Result<(), String> {
    scope.owner_only()?;
    let not_a_member = "that person is not a member of the team";
    let account = Uuid::parse_str(account).map_err(|_| not_a_member.to_owned())?;
    match db::role(&hub.db, scope.team.id, account).await? {
        None => return Err(not_a_member.into()),
        Some(Role::Owner) => return Err("the owner cannot be removed from the team".into()),
        Some(Role::Member) => {}
    }
    if !db::remove_member(&hub.db, scope.team.id, account).await? {
        return Err(not_a_member.into());
    }
    eprintln!("pinned-server: account {} removed {account} from team {}", scope.member, scope.team.id);
    Ok(())
}

/// The member leaves the team. The owner cannot leave: it removes the others first, or the team.
pub async fn leave(hub: &Arc<Hub>, scope: &Scope) -> Result<(), String> {
    if scope.role == Role::Owner {
        return Err("the owner cannot leave the team; remove the others first".into());
    }
    if !db::remove_member(&hub.db, scope.team.id, scope.member).await? {
        return Err("you are not a member of that team".into());
    }
    eprintln!("pinned-server: account {} left team {}", scope.member, scope.team.id);
    Ok(())
}
