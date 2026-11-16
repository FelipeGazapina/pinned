//! The teams' SQL, one function per query, in one section per file of `teams`. Each takes the
//! hub's `Db` and answers core types (`pinned_core::teams`) or the row types below.
//! The tables are in `db::SCHEMA`.

use crate::db::{err, Db};
use pinned_core::teams::{Answer, Bot, Call, Group, Role, Run, RunState, TeamProject};
use tokio_postgres::error::SqlState;
use uuid::Uuid;

/// A team the member is in, as `team_members` joined to `teams` holds it.
pub struct TeamRow {
    pub id: Uuid,
    pub name: String,
    pub owner: Uuid,
    pub role: Role,
}

/// A member of a team, with its email from `accounts`.
pub struct MemberRow {
    pub account: Uuid,
    pub email: String,
    pub role: Role,
}

// ---- mod.rs ----

/// The member's role on the team, or None when it is not a member (or the team does not exist).
pub async fn role(db: &Db, team: Uuid, member: Uuid) -> Result<Option<Role>, String> {
    let row = db.client().await?.query_opt("select role from team_members where team = $1 and member = $2", &[&team, &member]).await.map_err(err)?;
    Ok(row.map(|r| Role::parse(r.get(0))))
}

/// Every member's account id.
pub async fn member_ids(db: &Db, team: Uuid) -> Result<Vec<Uuid>, String> {
    let rows = db.client().await?.query("select member from team_members where team = $1", &[&team]).await.map_err(err)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Whether `account` is a team's account rather than a person's.
pub async fn is_team(db: &Db, account: Uuid) -> Result<bool, String> {
    let row = db.client().await?.query_opt("select kind = 'team' from accounts where id = $1", &[&account]).await.map_err(err)?;
    Ok(row.is_some_and(|r| r.get(0)))
}

// ---- membership.rs ----

/// Makes the team's account (kind 'team', email `team-<id>@teams.invalid`, password '!', which no
/// hash matches), its `teams` row and `owner` as its owner, in one statement, and answers its id.
pub async fn create_team(db: &Db, owner: Uuid, name: &str) -> Result<Uuid, String> {
    let id = Uuid::new_v4();
    let email = format!("team-{id}@teams.invalid");
    let made = db
        .client().await?
        .execute(
            "with account as (insert into accounts (id, email, password_hash, kind) values ($1, $2, '!', 'team') returning id),
                  team as (insert into teams (id, name, owner) select id, $3, $4 from account returning id)
             insert into team_members (team, member, role) select id, $4, 'owner' from team",
            &[&id, &email, &name, &owner],
        )
        .await
        .map_err(err)?;
    if made != 1 {
        return Err("the team was not made".into());
    }
    Ok(id)
}

/// The teams `member` is in, oldest first.
pub async fn teams_of(db: &Db, member: Uuid) -> Result<Vec<TeamRow>, String> {
    let rows = db
        .client().await?
        .query(
            "select t.id, t.name, t.owner, m.role from team_members m join teams t on t.id = m.team where m.member = $1 order by t.created_at",
            &[&member],
        )
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| TeamRow { id: r.get(0), name: r.get(1), owner: r.get(2), role: Role::parse(r.get(3)) }).collect())
}

/// Renames the team; false when it has no `teams` row.
pub async fn rename_team(db: &Db, team: Uuid, name: &str) -> Result<bool, String> {
    let renamed = db.client().await?.execute("update teams set name = $2 where id = $1", &[&team, &name]).await.map_err(err)?;
    Ok(renamed == 1)
}

/// The members with their emails, the owner first, then by join time.
pub async fn members(db: &Db, team: Uuid) -> Result<Vec<MemberRow>, String> {
    let rows = db
        .client()
        .await?
        .query(
            "select m.member, a.email, m.role from team_members m join accounts a on a.id = m.member
             where m.team = $1 order by m.role <> 'owner', m.joined_at, m.member",
            &[&team],
        )
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| MemberRow { account: r.get(0), email: r.get(1), role: Role::parse(r.get(2)) }).collect())
}

/// Each member's group handles, as `(member, handle)` rows, by handle.
pub async fn member_groups(db: &Db, team: Uuid) -> Result<Vec<(Uuid, String)>, String> {
    let rows = db.client().await?.query("select member, handle from team_group_members where team = $1 order by handle", &[&team]).await.map_err(err)?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// Keeps an invite by the sha256 of its code (`crate::db::invite_hash`), never the code.
pub async fn create_invite(db: &Db, team: Uuid, made_by: Uuid, code_hash: &[u8], hint: &str) -> Result<(), String> {
    db.client()
        .await?
        .execute("insert into team_invites (code_hash, team, made_by, hint) values ($1, $2, $3, $4)", &[&code_hash, &team, &made_by, &hint])
        .await
        .map_err(err)
        .map(drop)
}

/// Marks the unused, unrevoked invite `code_hash` used by `member` and adds `member` as a
/// member, in one statement; answers the team, or None when the code is unknown, used or revoked.
/// A member of the invite's team who joins again keeps its role and the invite stays unused, so
/// that case answers the team without spending it.
pub async fn take_invite(db: &Db, code_hash: &[u8], member: Uuid) -> Result<Option<Uuid>, String> {
    let client = db.client().await?;
    // A racing second spend finds `used_by` set when its row lock comes free, so only one join lands.
    let joined = client
        .query_opt(
            "with spent as (update team_invites set used_by = $2, used_at = now()
                 where code_hash = $1 and used_by is null and not revoked
                 and exists (select 1 from accounts where id = $2 and kind = 'person')
                 and not exists (select 1 from team_members where team = team_invites.team and member = $2)
                 returning team),
              added as (insert into team_members (team, member, role) select team, $2, 'member' from spent
                 on conflict (team, member) do nothing returning team)
             select team from added",
            &[&code_hash, &member],
        )
        .await
        .map_err(err)?;
    if let Some(row) = joined {
        return Ok(Some(row.get(0)));
    }
    let again = client
        .query_opt(
            "select i.team from team_invites i join team_members m on m.team = i.team and m.member = $2
             where i.code_hash = $1 and i.used_by is null and not i.revoked",
            &[&code_hash, &member],
        )
        .await
        .map_err(err)?;
    Ok(again.map(|r| r.get(0)))
}

/// Revokes the team's unused invites whose hint is `hint`; answers how many.
pub async fn revoke_invite(db: &Db, team: Uuid, hint: &str) -> Result<u64, String> {
    db.client().await?.execute("update team_invites set revoked = true where team = $1 and hint = $2 and used_by is null and not revoked", &[&team, &hint]).await.map_err(err)
}

/// Removes a member who is not the owner; false when there was none such. In one statement the
/// member's groups go by cascade, its attached projects are deleted here (`team_projects` cascades
/// from accounts, not from membership), and any group that named it as fallback forgets it.
pub async fn remove_member(db: &Db, team: Uuid, member: Uuid) -> Result<bool, String> {
    let row = db
        .client()
        .await?
        .query_one(
            "with gone as (delete from team_members where team = $1 and member = $2 and role <> 'owner' returning member),
                  projects as (delete from team_projects where team = $1 and member in (select member from gone)),
                  fallbacks as (update team_groups set fallback = null where team = $1 and fallback in (select member from gone))
             select exists (select 1 from gone)",
            &[&team, &member],
        )
        .await
        .map_err(err)?;
    Ok(row.get(0))
}

// ---- projects.rs ----

pub async fn attach_project(_db: &Db, _team: Uuid, _project: &TeamProject) -> Result<(), String> {
    Err("not built yet".into())
}

/// False when the team had no project `key`.
pub async fn detach_project(_db: &Db, _team: Uuid, _key: &str) -> Result<bool, String> {
    Err("not built yet".into())
}

pub async fn projects(_db: &Db, _team: Uuid) -> Result<Vec<TeamProject>, String> {
    Err("not built yet".into())
}

// ---- groups.rs ----

/// Who sets a group's members in `save_group`.
pub enum Members<'a> {
    /// Replaces the stored members (the owner's save).
    Replace(&'a [Uuid]),
    /// Keeps the members of a group that exists. A group made by this save starts with `new`
    /// (a member's save).
    Keep { new: &'a [Uuid] },
}

/// Every group by handle, with its members' account ids sorted. A fallback that is no longer a
/// member of the team reads as None.
pub async fn groups(db: &Db, team: Uuid) -> Result<Vec<Group>, String> {
    let rows = db
        .client()
        .await?
        .query(
            "select g.handle, g.about, g.slack_group,
                    case when exists (select 1 from team_members m where m.team = g.team and m.member = g.fallback) then g.fallback::text end,
                    array(select gm.member::text from team_group_members gm where gm.team = g.team and gm.handle = g.handle order by gm.member)
             from team_groups g where g.team = $1 order by g.handle",
            &[&team],
        )
        .await
        .map_err(err)?;
    Ok(rows
        .iter()
        .map(|r| Group { handle: r.get(0), about: r.get(1), slack_group: r.get(2), fallback: r.get(3), members: r.get(4) })
        .collect())
}

/// Inserts or replaces the group `group.handle` and its members, in one transaction
/// (`Db::transaction_client`). `group.fallback` must be a member's account id (checked by the
/// caller); `group.members` is ignored, `members` decides them. A member who is not on the team
/// fails the foreign key on `team_group_members`, which is answered as a readable error.
pub async fn save_group(db: &Db, team: Uuid, group: &Group, members: Members<'_>) -> Result<(), String> {
    let fallback = group.fallback.as_deref().map(Uuid::parse_str).transpose().map_err(|e| e.to_string())?;
    let mut held = db.transaction_client().await?;
    let tx = held.as_mut().expect("opened above").transaction().await.map_err(err)?;
    let created: bool = tx
        .query_one(
            "insert into team_groups (team, handle, about, fallback, slack_group) values ($1, $2, $3, $4, $5)
             on conflict (team, handle) do update set about = excluded.about, fallback = excluded.fallback, slack_group = excluded.slack_group
             returning xmax = 0",
            &[&team, &group.handle, &group.about, &fallback, &group.slack_group],
        )
        .await
        .map_err(err)?
        .get(0);
    let list: Option<&[Uuid]> = match members {
        Members::Replace(list) => Some(list),
        Members::Keep { new } if created => Some(new),
        Members::Keep { .. } => None,
    };
    if let Some(list) = list {
        tx.execute("delete from team_group_members where team = $1 and handle = $2", &[&team, &group.handle]).await.map_err(err)?;
        let added = tx
            .execute(
                "insert into team_group_members (team, handle, member) select $1, $2, m from unnest($3::uuid[]) as m",
                &[&team, &group.handle, &list],
            )
            .await;
        added.map_err(|e| {
            if e.code() == Some(&SqlState::FOREIGN_KEY_VIOLATION) {
                format!("@{} has someone who is not a member of this team", group.handle)
            } else {
                err(e)
            }
        })?;
    }
    tx.commit().await.map_err(err)?;
    Ok(())
}

/// Adds `member` to group `handle`, once; false when the group does not exist.
pub async fn join_group(db: &Db, team: Uuid, handle: &str, member: Uuid) -> Result<bool, String> {
    let row = db
        .client()
        .await?
        .query_one(
            "with g as (select handle from team_groups where team = $1 and handle = $2),
                  joined as (insert into team_group_members (team, handle, member) select $1, handle, $3 from g on conflict do nothing)
             select exists (select 1 from g)",
            &[&team, &handle, &member],
        )
        .await
        .map_err(err)?;
    Ok(row.get(0))
}

/// Removes `member` from group `handle`; false when it was not in the group.
pub async fn leave_group(db: &Db, team: Uuid, handle: &str, member: Uuid) -> Result<bool, String> {
    let removed = db
        .client()
        .await?
        .execute("delete from team_group_members where team = $1 and handle = $2 and member = $3", &[&team, &handle, &member])
        .await
        .map_err(err)?;
    Ok(removed == 1)
}

// ---- bots.rs ----

/// The `team_bots` rows: `Bot` with an empty `name` (the workflow's name is in the team's store).
pub async fn bots(_db: &Db, _team: Uuid) -> Result<Vec<Bot>, String> {
    Err("not built yet".into())
}

/// Inserts or replaces the row of `bot.workflow`, leaving `on_by` and `on_at` as they are.
pub async fn save_bot(_db: &Db, _team: Uuid, _bot: &Bot) -> Result<(), String> {
    Err("not built yet".into())
}

/// Sets `on_by`/`on_at` (on) or clears them (off); false when the bot has no row.
pub async fn turn_bot_on(_db: &Db, _team: Uuid, _workflow: &str, _on: bool, _by: Uuid) -> Result<bool, String> {
    Err("not built yet".into())
}

// ---- calls.rs ----

/// Inserts a run in `RunState::Working`; `machine` is the box it runs in, if known yet.
pub async fn insert_run(_db: &Db, _team: Uuid, _run: &Run, _machine: Option<&str>) -> Result<(), String> {
    Err("not built yet".into())
}

/// The team's runs, newest first, at most `limit`.
pub async fn runs(_db: &Db, _team: Uuid, _limit: i64) -> Result<Vec<Run>, String> {
    Err("not built yet".into())
}

pub async fn run(_db: &Db, _team: Uuid, _run: Uuid) -> Result<Option<Run>, String> {
    Err("not built yet".into())
}

/// The team's live run whose session runs on `machine`, if any.
pub async fn run_on_machine(_db: &Db, _team: Uuid, _machine: &str) -> Result<Option<Run>, String> {
    Err("not built yet".into())
}

/// Moves a run that has not ended to `state` with `controller`; sets `ended_at` and `outcome`
/// when `state` ends it. False when the run had already ended.
pub async fn set_run_state(_db: &Db, _team: Uuid, _run: Uuid, _state: RunState, _controller: Option<Uuid>, _outcome: Option<&str>) -> Result<bool, String> {
    Err("not built yet".into())
}

pub async fn insert_call(_db: &Db, _team: Uuid, _call: &Call) -> Result<(), String> {
    Err("not built yet".into())
}

/// The open calls, then the last 20 answered, newest first.
pub async fn calls(_db: &Db, _team: Uuid) -> Result<Vec<Call>, String> {
    Err("not built yet".into())
}

/// Sets the answer of a call nobody answered yet; false when someone already did.
pub async fn answer_call(_db: &Db, _team: Uuid, _call: Uuid, _answer: &Answer) -> Result<bool, String> {
    Err("not built yet".into())
}

/// Every team's unanswered calls older than `secs`, with their team, oldest first, for `tick`.
pub async fn calls_older_than(_db: &Db, _secs: u64, _escalated: bool) -> Result<Vec<(Uuid, Call)>, String> {
    Err("not built yet".into())
}

/// Marks the call escalated to the owner.
pub async fn escalate_call(_db: &Db, _call: Uuid) -> Result<(), String> {
    Err("not built yet".into())
}
