//! Who is on a team: making one, listing them, invites, members, leaving.

use super::{db, Scope};
use crate::db::{invite_code, invite_hash};
use crate::hub::{Account, Hub};
use pinned_core::teams::{CredentialState, Member, OpenInvite, Role, Team, TeamInvite};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The longest team name, in characters.
pub const NAME_CHARS: usize = 80;

/// The one answer to an invite code that is unknown, used or revoked, so a stranger learns nothing.
const NO_INVITE: &str = "that invite code is unknown, used or revoked";

/// The most open invites (unused and unrevoked) a team has at once. A code never expires: it is
/// good until it is used or revoked.
pub const MAX_OPEN_INVITES: i64 = 20;
/// Failed joins an account may make, and the hub in all, in `JOIN_WINDOW`.
pub const JOIN_FAILS_PER_ACCOUNT: usize = 10;
pub const JOIN_FAILS_PER_HUB: usize = 200;
pub const JOIN_WINDOW: Duration = Duration::from_secs(15 * 60);
const TOO_MANY_JOINS: &str = "too many invite codes tried; try again in 15 minutes";

/// When `join`s came that are not known to have landed, per account and for the whole hub, for
/// their limit. A try counts when it starts and is taken back (`forgive`) once it lands, so tries
/// racing each other can't all pass a full limit.
/// This limiter lives in memory only, on the `Hub`: it resets when the hub restarts, so a restart
/// gives every account and the hub a fresh window of tries.
#[derive(Default)]
pub struct JoinFails {
    hub: VecDeque<Instant>,
    by_account: HashMap<Uuid, VecDeque<Instant>>,
}

impl JoinFails {
    /// Counts a try of `account` at `now`, or refuses it when 10 of the account's own or 200 of the
    /// hub's in the last 15 minutes failed or are still running. A refused try does not count and
    /// must not reach the database.
    pub fn attempt(&mut self, account: Uuid, now: Instant) -> Result<(), String> {
        self.forget(now);
        let own = self.by_account.get(&account).map_or(0, VecDeque::len);
        if own >= JOIN_FAILS_PER_ACCOUNT || self.hub.len() >= JOIN_FAILS_PER_HUB {
            return Err(TOO_MANY_JOINS.into());
        }
        self.hub.push_back(now);
        self.by_account.entry(account).or_default().push_back(now);
        Ok(())
    }

    /// Takes back the try `attempt` counted at `now`: the join landed.
    pub fn forgive(&mut self, account: Uuid, now: Instant) {
        if let Some(i) = self.hub.iter().position(|t| *t == now) {
            self.hub.remove(i);
        }
        if let Some(at) = self.by_account.get_mut(&account) {
            if let Some(i) = at.iter().position(|t| *t == now) {
                at.remove(i);
            }
            if at.is_empty() {
                self.by_account.remove(&account);
            }
        }
    }

    /// Drops the tries older than the window, and the accounts left with none.
    fn forget(&mut self, now: Instant) {
        let old = |t: &Instant| now.duration_since(*t) >= JOIN_WINDOW;
        while self.hub.front().is_some_and(old) {
            self.hub.pop_front();
        }
        self.by_account.retain(|_, at| {
            while at.front().is_some_and(old) {
                at.pop_front();
            }
            !at.is_empty()
        });
    }
}

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
    Ok(credential_in(&hub.connectors(team).await?.get_settings()))
}

/// `credential` of the team's settings as already read.
pub fn credential_in(settings: &pinned_core::connectors::Settings) -> CredentialState {
    CredentialState {
        upstash: settings.upstash.is_some(),
        claude: settings.upstash.as_ref().is_some_and(|u| !u.claude_hint.is_empty()) || !settings.box_claude_tokens.is_empty(),
        github: settings.github.is_some(),
        slack: settings.slack.is_some() || !settings.slack_bots.is_empty(),
    }
}

/// A team as `member` sees it, from its `teams_of` row.
async fn view(hub: &Arc<Hub>, row: db::TeamRow) -> Result<Team, String> {
    let account = hub.account(row.id).await?;
    // A team whose settings can't be read is still listed, with no key shown as set: its owner
    // sees why while its waiting line is held for it (`line_held`).
    let credential = match credential(hub, &account).await {
        Err(e) if e == crate::db::BUSY => return Err(e),
        read => read.unwrap_or_default(),
    };
    let line_held = row.line_held.filter(|_| row.role == Role::Owner);
    Ok(Team { id: row.id.to_string(), name: row.name, owner: row.owner.to_string(), role: row.role, credential, line_held })
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
    Ok(Team { id: id.to_string(), name: name.to_owned(), owner: person.to_string(), role: pinned_core::teams::Role::Owner, credential: CredentialState::default(), line_held: None })
}

/// Joins the team of invite `code` as a member, once: `db::take_invite` with the code's sha256
/// (`crate::db::invite_hash`, which ignores case, spaces and dashes). Unknown, used and revoked codes all
/// answer the same error; a code never expires, however old. A member of the team already keeps
/// its role. Failed tries are limited per account and per hub (`JoinFails`): past the limit the code
/// is not looked up. A join that lands does not count. Answers the team as `list` shows it.
pub async fn join(hub: &Arc<Hub>, person: Uuid, code: &str) -> Result<Team, String> {
    let tried = Instant::now();
    hub.team_join_fails.lock().unwrap().attempt(person, tried)?;
    let team = match db::take_invite(&hub.db, &invite_hash(code), person).await {
        Ok(Some(team)) => team,
        Ok(None) => return Err(NO_INVITE.into()),
        Err(e) => {
            // The hub failed, not the code: that is no failed guess.
            hub.team_join_fails.lock().unwrap().forgive(person, tried);
            return Err(e);
        }
    };
    hub.team_join_fails.lock().unwrap().forgive(person, tried);
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

/// Owner only. A new single-use invite code (`crate::db::invite_code`), good until it is used or
/// revoked, with no expiry; the hub keeps only its sha256 and the hint (its first group), and shows the code this once.
/// A team has at most `MAX_OPEN_INVITES` open invites.
pub async fn invite(hub: &Arc<Hub>, scope: &Scope) -> Result<TeamInvite, String> {
    scope.owner_only()?;
    let code = invite_code();
    let hint = code[..4].to_owned();
    if !db::create_invite(&hub.db, scope.team.id, scope.member, &invite_hash(&code), &hint, MAX_OPEN_INVITES).await? {
        return Err(format!("this team has {MAX_OPEN_INVITES} open invite codes; revoke some first"));
    }
    eprintln!("pinned-server: account {} made an invite to team {}", scope.member, scope.team.id);
    Ok(TeamInvite { code, hint })
}

/// Owner only. The open invites (not used, not revoked), newest first: each one's hint, when it
/// was made and who made it, never its code.
pub async fn invites(hub: &Arc<Hub>, scope: &Scope) -> Result<Vec<OpenInvite>, String> {
    scope.owner_only()?;
    Ok(db::open_invites(&hub.db, scope.team.id)
        .await?
        .into_iter()
        .map(|row| {
            let name = row.email.as_deref().and_then(|e| e.split('@').next()).filter(|n| !n.is_empty()).map(str::to_owned);
            OpenInvite { hint: row.hint, created: row.created, made_by: name.unwrap_or_else(|| row.made_by.to_string()) }
        })
        .collect())
}

/// Owner only. Revokes the open invites whose hint is `hint`; an error when there was none.
pub async fn revoke_invite(hub: &Arc<Hub>, scope: &Scope, hint: &str) -> Result<(), String> {
    scope.owner_only()?;
    let hint = hint.trim().to_ascii_uppercase();
    match db::revoke_invite(&hub.db, scope.team.id, &hint).await? {
        0 => Err("there is no unused invite with that hint".into()),
        _ => Ok(()),
    }
}

/// Owner only. Removes member `account` (never the owner). Its groups lose it by cascade; the
/// projects it attached go with it and the bots it owned lose their owner (`db::remove_member`);
/// its runs keep going. An `account` that is not a member of this team, however it is wrong, gets
/// `members_only`'s one message. A live run it controlled goes back to its bot, and a call it was
/// the only one asked goes to the owner (`calls::member_gone`).
pub async fn remove_member(hub: &Arc<Hub>, scope: &Scope, account: &str) -> Result<(), String> {
    scope.owner_only()?;
    let account = super::members_only(hub, scope.team.id, &[account]).await?[0];
    if db::role(&hub.db, scope.team.id, account).await? == Some(Role::Owner) {
        return Err("the owner cannot be removed from the team".into());
    }
    let asking = super::calls::calls_asking(hub, scope.team.id, account).await?;
    if !db::remove_member(&hub.db, scope.team.id, account).await? {
        return Err(format!("no member {account} on this team"));
    }
    super::calls::member_gone(hub, &scope.team, account, asking).await;
    eprintln!("pinned-server: account {} removed {account} from team {}", scope.member, scope.team.id);
    Ok(())
}

/// The member leaves the team, as `remove_member` removes one: its projects go, it stops being a
/// group's fallback, its bots lose their owner, a live run it controlled goes back to its bot and a
/// call it was the only one asked goes to the owner. The owner cannot leave: it removes the others first, or the team.
pub async fn leave(hub: &Arc<Hub>, scope: &Scope) -> Result<(), String> {
    if scope.role == Role::Owner {
        return Err("the owner cannot leave the team; remove the others first".into());
    }
    let asking = super::calls::calls_asking(hub, scope.team.id, scope.member).await?;
    if !db::remove_member(&hub.db, scope.team.id, scope.member).await? {
        return Err("you are not a member of that team".into());
    }
    super::calls::member_gone(hub, &scope.team, scope.member, asking).await;
    eprintln!("pinned-server: account {} left team {}", scope.member, scope.team.id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFUSED: &str = "too many invite codes tried; try again in 15 minutes";

    #[test]
    fn an_account_may_fail_10_joins_in_15_minutes_and_a_join_that_lands_does_not_count() {
        let (start, account) = (Instant::now(), Uuid::new_v4());
        let mut fails = JoinFails::default();
        for _ in 0..9 {
            fails.attempt(account, start).unwrap();
        }
        fails.attempt(account, start).unwrap();
        fails.forgive(account, start);
        fails.attempt(account, start).unwrap();
        assert_eq!(fails.attempt(account, start), Err(REFUSED.into()), "the 10 failed ones fill the account's limit");
        assert_eq!(fails.attempt(account, start + Duration::from_secs(899)), Err(REFUSED.into()), "a refused try did not count and did not move the window");
        assert_eq!(fails.attempt(Uuid::new_v4(), start), Ok(()), "another account is not held by it");
        fails.attempt(account, start + JOIN_WINDOW).unwrap();
    }

    #[test]
    fn the_hub_stops_at_200_failed_joins_in_15_minutes_whoever_tried() {
        let start = Instant::now();
        let mut fails = JoinFails::default();
        for _ in 0..20 {
            let account = Uuid::new_v4();
            for _ in 0..10 {
                fails.attempt(account, start).unwrap();
            }
        }
        assert_eq!(fails.attempt(Uuid::new_v4(), start), Err(REFUSED.into()));
        fails.attempt(Uuid::new_v4(), start + JOIN_WINDOW).unwrap();
    }
}
