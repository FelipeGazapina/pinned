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

fn bot_row(r: &tokio_postgres::Row) -> Result<Bot, String> {
    let calls: serde_json::Value = r.get("calls");
    let limits: serde_json::Value = r.get("limits");
    let owner: Option<Uuid> = r.get("owner");
    let on_by: Option<Uuid> = r.get("on_by");
    let on: bool = r.get("on");
    Ok(Bot {
        workflow: r.get("workflow"),
        name: String::new(),
        owner: owner.map(|o| o.to_string()),
        calls: serde_json::from_value(calls).map_err(|e| format!("a bot's call rules are not readable: {e}"))?,
        limits: serde_json::from_value(limits).map_err(|e| format!("a bot's limits are not readable: {e}"))?,
        on,
        on_by: on_by.map(|o| o.to_string()),
        once: r.get("once"),
    })
}

const BOT_COLUMNS: &str = "workflow, owner, calls, limits, on_by, on_at is not null as on, once";

/// The `team_bots` rows: `Bot` with an empty `name` (the workflow's name is in the team's store).
pub async fn bots(db: &Db, team: Uuid) -> Result<Vec<Bot>, String> {
    let rows = db.client().await?.query(&format!("select {BOT_COLUMNS} from team_bots where team = $1 order by workflow"), &[&team]).await.map_err(err)?;
    rows.iter().map(bot_row).collect()
}

/// The row of one workflow, if it is a bot.
pub async fn bot(db: &Db, team: Uuid, workflow: &str) -> Result<Option<Bot>, String> {
    let row = db.client().await?.query_opt(&format!("select {BOT_COLUMNS} from team_bots where team = $1 and workflow = $2"), &[&team, &workflow]).await.map_err(err)?;
    row.as_ref().map(bot_row).transpose()
}

/// Inserts or replaces the row of `bot.workflow`, leaving `on_by` and `on_at` as they are.
pub async fn save_bot(db: &Db, team: Uuid, bot: &Bot) -> Result<(), String> {
    let owner = bot.owner.as_deref().map(Uuid::parse_str).transpose().map_err(|_| "the bot's owner is not an account id".to_owned())?;
    let calls = serde_json::to_value(&bot.calls).map_err(|e| e.to_string())?;
    let limits = serde_json::to_value(&bot.limits).map_err(|e| e.to_string())?;
    db.client()
        .await?
        .execute(
            "insert into team_bots (team, workflow, owner, calls, limits, once) values ($1, $2, $3, $4, $5, $6)
             on conflict (team, workflow) do update set owner = excluded.owner, calls = excluded.calls, limits = excluded.limits, once = excluded.once",
            &[&team, &bot.workflow, &owner, &calls, &limits, &bot.once],
        )
        .await
        .map_err(err)?;
    Ok(())
}

/// Sets `on_by`/`on_at` (on) or clears them (off); false when the bot has no row.
pub async fn turn_bot_on(db: &Db, team: Uuid, workflow: &str, on: bool, by: Uuid) -> Result<bool, String> {
    let client = db.client().await?;
    let changed = match on {
        true => client.execute("update team_bots set on_by = $3, on_at = now() where team = $1 and workflow = $2", &[&team, &workflow, &by]).await,
        false => client.execute("update team_bots set on_by = null, on_at = null where team = $1 and workflow = $2", &[&team, &workflow]).await,
    };
    Ok(changed.map_err(err)? == 1)
}

/// Deletes the row of a workflow; false when it was no bot.
pub async fn delete_bot(db: &Db, team: Uuid, workflow: &str) -> Result<bool, String> {
    let gone = db.client().await?.execute("delete from team_bots where team = $1 and workflow = $2", &[&team, &workflow]).await.map_err(err)?;
    Ok(gone == 1)
}

/// The handles of the team's groups.
pub async fn group_handles(db: &Db, team: Uuid) -> Result<Vec<String>, String> {
    let rows = db.client().await?.query("select handle from team_groups where team = $1", &[&team]).await.map_err(err)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Whether `member` is in the team's group `handle`.
pub async fn in_group(db: &Db, team: Uuid, handle: &str, member: Uuid) -> Result<bool, String> {
    let row = db.client().await?.query_opt("select 1 from team_group_members where team = $1 and handle = $2 and member = $3", &[&team, &handle, &member]).await.map_err(err)?;
    Ok(row.is_some())
}

/// Records the box a run opened (the id of its machine), once it is known.
pub async fn set_run_box(db: &Db, team: Uuid, run: Uuid, machine: &str) -> Result<(), String> {
    db.client().await?.execute("update team_runs set box = $3 where team = $1 and id = $2", &[&team, &run, &machine]).await.map_err(err)?;
    Ok(())
}

/// Forgets a run that never started.
pub async fn delete_run(db: &Db, team: Uuid, run: Uuid) -> Result<(), String> {
    db.client().await?.execute("delete from team_runs where team = $1 and id = $2", &[&team, &run]).await.map_err(err)?;
    Ok(())
}

// ---- calls.rs ----

/// Inserts a run in `RunState::Working`; `machine` is the box it runs in, if known yet.
pub async fn insert_run(db: &Db, team: Uuid, run: &Run, machine: Option<&str>) -> Result<(), String> {
    let id = Uuid::parse_str(&run.id).map_err(|_| "the run's id is not a uuid".to_owned())?;
    let started_by = serde_json::Value::String(run.started_by.clone());
    db.client()
        .await?
        .execute(
            "insert into team_runs (id, team, workflow, box, started_by, state, started_at) values ($1, $2, $3, $4, $5, $6, to_timestamp($7::bigint))",
            &[&id, &team, &run.workflow, &machine, &started_by, &run.state.as_str(), &run.started_at],
        )
        .await
        .map_err(err)?;
    Ok(())
}

/// The team's runs, newest first, at most `limit`.
pub async fn runs(db: &Db, team: Uuid, limit: i64) -> Result<Vec<Run>, String> {
    let rows = db
        .client()
        .await?
        .query(
            "select id, workflow, state, controller, cost_usd, active_secs, extract(epoch from started_at)::bigint, extract(epoch from ended_at)::bigint, outcome, started_by
             from team_runs where team = $1 order by started_at desc, id limit $2",
            &[&team, &limit],
        )
        .await
        .map_err(err)?;
    Ok(rows
        .iter()
        .map(|r| {
            let (id, controller, started_by): (Uuid, Option<Uuid>, serde_json::Value) = (r.get(0), r.get(3), r.get(9));
            let active: i64 = r.get(5);
            Run {
                id: id.to_string(),
                workflow: r.get(1),
                state: RunState::parse(r.get(2)).unwrap_or(RunState::Failed),
                controller: controller.map(|c| c.to_string()),
                cost_usd: r.get(4),
                active_secs: active.max(0) as u64,
                started_at: r.get(6),
                ended_at: r.get(7),
                outcome: r.get(8),
                started_by: started_by.as_str().unwrap_or_default().to_owned(),
            }
        })
        .collect())
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
