//! The teams' SQL, one function per query, in one section per file of `teams`. Each takes the
//! hub's `Db` and answers core types (`pinned_core::teams`) or the row types below.
//! The tables are in `db::SCHEMA`.

use crate::db::{err, Db};
use pinned_core::teams::{Answer, Bot, Call, Group, Role, Run, RunState, TeamProject};
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
pub async fn rename_team(_db: &Db, _team: Uuid, _name: &str) -> Result<bool, String> {
    Err("not built yet".into())
}

/// The members with their emails, the owner first.
pub async fn members(_db: &Db, _team: Uuid) -> Result<Vec<MemberRow>, String> {
    Err("not built yet".into())
}

/// Keeps an invite by the sha256 of its code (`crate::db::hash`), never the code.
pub async fn create_invite(_db: &Db, _team: Uuid, _made_by: Uuid, _code_hash: &[u8], _hint: &str) -> Result<(), String> {
    Err("not built yet".into())
}

/// Marks the unused, unrevoked invite `code_hash` used by `member` and adds `member` as a
/// member, in one statement; answers the team, or None when the code is unknown, used or revoked.
pub async fn take_invite(_db: &Db, _code_hash: &[u8], _member: Uuid) -> Result<Option<Uuid>, String> {
    Err("not built yet".into())
}

/// Revokes the team's unused invites whose hint is `hint`; answers how many.
pub async fn revoke_invite(_db: &Db, _team: Uuid, _hint: &str) -> Result<u64, String> {
    Err("not built yet".into())
}

/// Removes a member who is not the owner; false when there was none such.
pub async fn remove_member(_db: &Db, _team: Uuid, _member: Uuid) -> Result<bool, String> {
    Err("not built yet".into())
}

// ---- projects.rs ----

/// Keeps the project under the team, attached by `project.member`. A key the team already has
/// stays as it is, with the name, repo and member of whoever attached it first.
pub async fn attach_project(db: &Db, team: Uuid, project: &TeamProject) -> Result<(), String> {
    let member = Uuid::parse_str(&project.member).map_err(|_| "that member is not an account id".to_string())?;
    db.client().await?
        .execute(
            "insert into team_projects (team, key, name, repo, member) values ($1, $2, $3, $4, $5) on conflict (team, key) do nothing",
            &[&team, &project.key, &project.name, &project.repo, &member],
        )
        .await
        .map_err(err)?;
    Ok(())
}

/// False when the team had no project `key`.
pub async fn detach_project(db: &Db, team: Uuid, key: &str) -> Result<bool, String> {
    let gone = db.client().await?.execute("delete from team_projects where team = $1 and key = $2", &[&team, &key]).await.map_err(err)?;
    Ok(gone == 1)
}

/// Like `detach_project`, but only a project that `member` attached; false when there was none such.
pub async fn detach_project_attached_by(db: &Db, team: Uuid, key: &str, member: Uuid) -> Result<bool, String> {
    let gone = db.client().await?.execute("delete from team_projects where team = $1 and key = $2 and member = $3", &[&team, &key, &member]).await.map_err(err)?;
    Ok(gone == 1)
}

/// The team's projects, in the order they were attached.
pub async fn projects(db: &Db, team: Uuid) -> Result<Vec<TeamProject>, String> {
    let rows = db
        .client().await?
        .query("select key, name, repo, member::text from team_projects where team = $1 order by attached_at, key", &[&team])
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| TeamProject { key: r.get(0), name: r.get(1), repo: r.get(2), member: r.get(3) }).collect())
}

// ---- groups.rs ----

/// Every group with its members' account ids, by handle.
pub async fn groups(_db: &Db, _team: Uuid) -> Result<Vec<Group>, String> {
    Err("not built yet".into())
}

/// Inserts or replaces the group `group.handle` and its members, in one transaction
/// (`Db::transaction_client`).
pub async fn save_group(_db: &Db, _team: Uuid, _group: &Group) -> Result<(), String> {
    Err("not built yet".into())
}

/// False when the group does not exist.
pub async fn join_group(_db: &Db, _team: Uuid, _handle: &str, _member: Uuid) -> Result<bool, String> {
    Err("not built yet".into())
}

pub async fn leave_group(_db: &Db, _team: Uuid, _handle: &str, _member: Uuid) -> Result<bool, String> {
    Err("not built yet".into())
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
