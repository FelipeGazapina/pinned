//! The teams' SQL, one function per query, in one section per file of `teams`. Each takes the
//! hub's `Db` and answers core types (`pinned_core::teams`) or the row types below.
//! The tables are in `db::SCHEMA`.

use crate::db::{err, Db};
use tokio_postgres::Client;
use pinned_core::agent::Subject;
use pinned_core::preauth::team_hook::GO_AHEAD;
use pinned_core::teams::{Answer, Bot, BotLimits, Call, Group, Role, Run, RunState, SkipReason, Skipped, TeamProject, Who};
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

/// Which of `ids` are members of the team.
pub async fn members_among(db: &Db, team: Uuid, ids: &[Uuid]) -> Result<Vec<Uuid>, String> {
    let rows = db.client().await?.query("select member from team_members where team = $1 and member = any($2)", &[&team, &ids]).await.map_err(err)?;
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
/// The owner is a person: a team never owns a team.
pub async fn create_team(db: &Db, owner: Uuid, name: &str) -> Result<Uuid, String> {
    let id = Uuid::new_v4();
    let email = format!("team-{id}@teams.invalid");
    let made = db
        .client().await?
        .execute(
            "with account as (insert into accounts (id, email, password_hash, kind)
                     select $1, $2, '!', 'team' where exists (select 1 from accounts where id = $4 and kind = 'person') returning id),
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

/// Keeps an invite by the sha256 of its code (`crate::db::invite_hash`), never the code, unless the
/// team already has `max_open` open invites (unused and unrevoked: a code never expires): false
/// then. The count and the insert run under one lock per team, so two racing invites never
/// pass the cap together.
pub async fn create_invite(db: &Db, team: Uuid, made_by: Uuid, code_hash: &[u8], hint: &str, max_open: i64) -> Result<bool, String> {
    let mut held = db.transaction_client().await?;
    let tx = held.as_mut().expect("opened above").transaction().await.map_err(err)?;
    tx.execute("select pg_advisory_xact_lock(hashtext($1::text))", &[&format!("team_invites:{team}")]).await.map_err(err)?;
    let open: i64 = tx
        .query_one(
            "select count(*) from team_invites where team = $1 and used_by is null and not revoked",
            &[&team],
        )
        .await
        .map_err(err)?
        .get(0);
    if open >= max_open {
        return Ok(false);
    }
    tx.execute("insert into team_invites (code_hash, team, made_by, hint) values ($1, $2, $3, $4)", &[&code_hash, &team, &made_by, &hint]).await.map_err(err)?;
    tx.commit().await.map_err(err)?;
    Ok(true)
}

/// Marks the unused, unrevoked invite `code_hash`, however old, used by `member` and adds
/// `member` as a member, in one statement; answers the team, or None when the code is unknown,
/// used or revoked. It reads `team_invites` only: an account signup code
/// (`invites`) is never a team code.
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

/// Revokes the team's open invites (unused and unrevoked, however old) whose hint is `hint`;
/// answers how many.
pub async fn revoke_invite(db: &Db, team: Uuid, hint: &str) -> Result<u64, String> {
    db.client()
        .await?
        .execute(
            "update team_invites set revoked = true
             where team = $1 and hint = $2 and used_by is null and not revoked",
            &[&team, &hint],
        )
        .await
        .map_err(err)
}

/// Removes a member who is not the owner; false when there was none such. In one statement the
/// member's groups go by cascade, its attached projects are deleted here (`team_projects` cascades
/// from accounts, not from membership), any group that named it as fallback forgets it, and the
/// bots it owns keep running with no owner. `leave` is this same statement.
pub async fn remove_member(db: &Db, team: Uuid, member: Uuid) -> Result<bool, String> {
    let row = db
        .client()
        .await?
        .query_one(
            "with gone as (delete from team_members where team = $1 and member = $2 and role <> 'owner' returning member),
                  projects as (delete from team_projects where team = $1 and member in (select member from gone)),
                  fallbacks as (update team_groups set fallback = null where team = $1 and fallback in (select member from gone)),
                  bots as (update team_bots set owner = null where team = $1 and owner in (select member from gone))
             select exists (select 1 from gone)",
            &[&team, &member],
        )
        .await
        .map_err(err)?;
    Ok(row.get(0))
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
        off_note: r.get("off_note"),
        skipped: vec![],
    })
}

const BOT_COLUMNS: &str = "workflow, owner, calls, limits, on_by, on_at is not null as on, once, off_note";

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

/// Sets `on_by`/`on_at` and clears `off_note` (on), or clears them (off); false when the bot has no row.
pub async fn turn_bot_on(db: &Db, team: Uuid, workflow: &str, on: bool, by: Uuid) -> Result<bool, String> {
    let client = db.client().await?;
    let changed = match on {
        true => client.execute("update team_bots set on_by = $3, on_at = now(), off_note = null where team = $1 and workflow = $2", &[&team, &workflow, &by]).await,
        false => client.execute("update team_bots set on_by = null, on_at = null where team = $1 and workflow = $2", &[&team, &workflow]).await,
    };
    Ok(changed.map_err(err)? == 1)
}

/// Turns the bot off with `note`, which members see on it; false when it was not on.
pub async fn turn_bot_off_noted(db: &Db, team: Uuid, workflow: &str, note: &str) -> Result<bool, String> {
    let changed = db
        .client()
        .await?
        .execute("update team_bots set on_by = null, on_at = null, off_note = $3 where team = $1 and workflow = $2 and on_at is not null", &[&team, &workflow, &note])
        .await
        .map_err(err)?;
    Ok(changed == 1)
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

/// Records the box a live run opened (the id of its machine), once it is known. False when the
/// run ended meanwhile, as when someone stopped it while its box booted: it gets no box.
pub async fn set_run_box(db: &Db, team: Uuid, run: Uuid, machine: &str) -> Result<bool, String> {
    let set = db
        .client()
        .await?
        .execute(&format!("update team_runs set box = $3 where team = $1 and id = $2 and state in {LIVE}"), &[&team, &run, &machine])
        .await
        .map_err(err)?;
    Ok(set == 1)
}

/// Records the subject a run's box is opened for, so the box's machine is set on the run before
/// its session starts (`claim_run_box`).
pub async fn set_run_subject(db: &Db, team: Uuid, run: Uuid, subject: &Subject) -> Result<(), String> {
    db.client().await?.execute("update team_runs set subject = $3 where team = $1 and id = $2", &[&team, &run, &json(subject)?]).await.map_err(err)?;
    Ok(())
}

/// Holds a place for run `run` of an admitted trigger's event on `subject`, before anything
/// outside pinned hears of the event: the cap counts it, and its session's `open` takes it.
/// Written on `held`, the connection of the team's turn, so it commits with the turn.
pub async fn insert_reserved(held: &Client, team: Uuid, run: &Run, subject: &Subject) -> Result<(), String> {
    let id = Uuid::parse_str(&run.id).map_err(|_| "a run's id is a uuid")?;
    let started_by = serde_json::Value::String(run.started_by.clone());
    held
        .execute(
            "insert into team_runs (id, team, workflow, started_by, state, subject, reserved, reserved_at) values ($1, $2, $3, $4, 'working', $5, true, now())",
            &[&id, &team, &run.workflow, &started_by, &json(subject)?],
        )
        .await
        .map_err(err)
        .map(drop)
}

/// The live run of bot `workflow` that holds a place for `subject` and has opened no session yet.
pub async fn reserved_run_for(db: &Db, team: Uuid, workflow: &str, subject: &Subject) -> Result<Option<Uuid>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!("select id from team_runs where team = $1 and workflow = $2 and subject = $3 and reserved and state in {LIVE} limit 1"),
            &[&team, &workflow, &json(subject)?],
        )
        .await
        .map_err(err)?;
    Ok(row.map(|r| r.get(0)))
}

/// Whether the newest run of bot `workflow` on `subject` ended while it still only held its place:
/// its session never opened (the backstop, a stop or a failed start ended it).
pub async fn last_run_never_opened(db: &Db, team: Uuid, workflow: &str, subject: &Subject) -> Result<bool, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!("select reserved and state not in {LIVE} from team_runs where team = $1 and workflow = $2 and subject = $3 order by started_at desc limit 1"),
            &[&team, &workflow, &json(subject)?],
        )
        .await
        .map_err(err)?;
    Ok(row.is_some_and(|r| r.get(0)))
}

/// Whether live run `run` still only holds its place.
pub async fn is_reserved(db: &Db, team: Uuid, run: Uuid) -> Result<bool, String> {
    let row = db.client().await?.query_opt("select reserved from team_runs where team = $1 and id = $2", &[&team, &run]).await.map_err(err)?;
    Ok(row.is_some_and(|r| r.get(0)))
}

/// Run `run` opened its session on `machine`: it no longer only holds a place. Written only
/// while the run is live and still reserved, like `set_run_box`: false when it ended meanwhile,
/// as when someone stopped it or the backstop ended it while its box booted, so it gets no box.
pub async fn open_reserved(db: &Db, team: Uuid, run: Uuid, machine: &str) -> Result<bool, String> {
    let set = db
        .client()
        .await?
        .execute(
            &format!("update team_runs set box = $3, reserved = false, reserved_at = null, opened_at = now() where team = $1 and id = $2 and reserved and state in {LIVE}"),
            &[&team, &run, &machine],
        )
        .await
        .map_err(err)?;
    Ok(set == 1)
}

/// What became of the place reserved run `run` held: still held (live and reserved), its session
/// opened, or ended before its session opened (ended while reserved, or deleted by a failed start).
pub enum Reservation {
    Held,
    Opened,
    Ended,
}

/// `Reservation` of run `run`, read from its own row only.
pub async fn reservation_of(db: &Db, team: Uuid, run: Uuid) -> Result<Reservation, String> {
    let row = db
        .client()
        .await?
        .query_opt(&format!("select reserved, state in {LIVE} from team_runs where team = $1 and id = $2"), &[&team, &run])
        .await
        .map_err(err)?;
    Ok(match row.map(|r| (r.get::<_, bool>(0), r.get::<_, bool>(1))) {
        Some((true, true)) => Reservation::Held,
        Some((false, _)) => Reservation::Opened,
        Some((true, false)) | None => Reservation::Ended,
    })
}

/// Reserved run `run`'s session did not open. One statement on the run's row decides whether this
/// start is counted as failed: a run still live and still reserved is deleted, and one its box's
/// failure ended (`state = 'failed'`, any outcome but `never_opened`, the backstop's) is marked
/// counted (`reserved_at = null`). Answers true when either write returned the row. A run the
/// backstop ended, which counted `NeverStarted`, or one that was stopped, matches neither, so
/// `StartFailed` and `NeverStarted` are never both counted for one run, and the backstop's ended
/// row is never deleted.
pub async fn fail_reserved_start(db: &Db, team: Uuid, run: Uuid, never_opened: &str) -> Result<bool, String> {
    let row = db
        .client()
        .await?
        .query_one(
            &format!(
                "with gone as (delete from team_runs where team = $1 and id = $2 and reserved and state in {LIVE} returning id),
                      failed as (update team_runs set reserved_at = null
                                 where team = $1 and id = $2 and reserved and reserved_at is not null and state = 'failed' and outcome is distinct from $3
                                 returning id)
                 select (select count(*) from gone) + (select count(*) from failed)"
            ),
            &[&team, &run, &never_opened],
        )
        .await
        .map_err(err)?;
    Ok(row.get::<_, i64>(0) > 0)
}

// ---- calls.rs ----

use pinned_core::teams::CallRules;
use tokio_postgres::Row;

/// The seconds a `team_runs` row waited on people: its closed waits and the open one. In an
/// `update`'s `set`, it reads the row as it was, so a run moved out of `waiting` adds its wait.
macro_rules! waited {
    () => {
        "(waiting_secs + case when state = 'waiting' and waiting_since is not null then extract(epoch from now() - waiting_since)::bigint else 0 end)"
    };
}

/// A `team_runs` row as `run_of` reads it. Its waiting time is its closed waits (`waiting_secs`)
/// and the open one (since `waiting_since`); its active time is the rest of the time from its
/// session's open (`opened_at`, or its start for a run that opened with its row) to its end, or
/// now while it is live: 0 while its place is only reserved.
const RUN_COLUMNS: &str = concat!(
    "id, workflow, state, controller, cost_usd, case when reserved then 0 else greatest(0, extract(epoch from coalesce(ended_at, now()) - coalesce(opened_at, started_at))::bigint - ",
    waited!(),
    ") end, extract(epoch from started_at)::bigint, extract(epoch from ended_at)::bigint, outcome, started_by #>> '{}', ",
    waited!()
);

/// A `team_calls` row `c` with its run `r`, as `call_of` reads it.
const CALL_COLUMNS: &str = "c.id, c.run, r.workflow, c.situation, c.asked, c.question, c.choices, extract(epoch from c.created_at)::bigint, \
     c.escalated_at is not null, c.answer, c.outcome, c.team, c.step_hash";

/// The run states a run that has not ended is in.
const LIVE: &str = "('working', 'waiting', 'taken_over')";

fn run_of(r: &Row) -> Run {
    Run {
        id: r.get::<_, Uuid>(0).to_string(),
        workflow: r.get(1),
        // A state this hub does not know reads as failed, which nothing moves again.
        state: RunState::parse(r.get(2)).unwrap_or(RunState::Failed),
        controller: r.get::<_, Option<Uuid>>(3).map(|c| c.to_string()),
        cost_usd: r.get(4),
        active_secs: r.get::<_, i64>(5).max(0) as u64,
        started_at: r.get(6),
        ended_at: r.get(7),
        outcome: r.get(8),
        started_by: r.get::<_, Option<String>>(9).unwrap_or_default(),
        waiting_secs: r.get::<_, i64>(10).max(0) as u64,
    }
}

/// A call as the team reads it, with its outcome (`answered`, `passed`, `taken-over`,
/// `stopped` or `gave-up`), which only a closed call has.
pub struct CallRow {
    pub team: Uuid,
    pub call: Call,
    pub outcome: Option<String>,
    /// The hash of the exact step the team-box hook asked about, if it was the hook.
    pub step: Option<String>,
}

impl CallRow {
    pub fn closed(&self) -> bool {
        self.outcome.is_some()
    }
}

fn call_of(r: &Row) -> Result<CallRow, String> {
    let situation = r.get::<_, String>(3);
    let call = Call {
        id: r.get::<_, Uuid>(0).to_string(),
        run: r.get::<_, Uuid>(1).to_string(),
        workflow: r.get(2),
        situation: serde_json::from_value(serde_json::Value::String(situation.clone())).map_err(|_| format!("a call has the unknown situation {situation}"))?,
        asked: serde_json::from_value(r.get(4)).map_err(|e| format!("a call's asked does not read: {e}"))?,
        question: r.get(5),
        choices: serde_json::from_value(r.get(6)).map_err(|e| format!("a call's choices do not read: {e}"))?,
        created_at: r.get(7),
        escalated: r.get(8),
        answer: r.get::<_, Option<serde_json::Value>>(9).map(serde_json::from_value).transpose().map_err(|e| format!("a call's answer does not read: {e}"))?,
    };
    Ok(CallRow { team: r.get(11), call, outcome: r.get(10), step: r.get(12) })
}

fn calls_of(rows: &[Row]) -> Result<Vec<CallRow>, String> {
    rows.iter().map(call_of).collect()
}

fn json(value: &impl serde::Serialize) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// Inserts a run in `RunState::Working`; `machine` is the box it runs in, if known yet.
pub async fn insert_run(db: &Db, team: Uuid, run: &Run, machine: Option<&str>) -> Result<(), String> {
    let id = Uuid::parse_str(&run.id).map_err(|_| "a run's id is a uuid")?;
    let started_by = serde_json::Value::String(run.started_by.clone());
    db.client()
        .await?
        .execute(
            "insert into team_runs (id, team, workflow, box, started_by, state) values ($1, $2, $3, $4, $5, 'working')",
            &[&id, &team, &run.workflow, &machine, &started_by],
        )
        .await
        .map_err(err)
        .map(drop)
}

/// The team's runs, newest first, at most `limit`.
pub async fn runs(db: &Db, team: Uuid, limit: i64) -> Result<Vec<Run>, String> {
    let rows = db
        .client()
        .await?
        .query(&format!("select {RUN_COLUMNS} from team_runs where team = $1 order by started_at desc, id limit $2"), &[&team, &limit])
        .await
        .map_err(err)?;
    Ok(rows.iter().map(run_of).collect())
}

pub async fn run(db: &Db, team: Uuid, run: Uuid) -> Result<Option<Run>, String> {
    let row = db.client().await?.query_opt(&format!("select {RUN_COLUMNS} from team_runs where team = $1 and id = $2"), &[&team, &run]).await.map_err(err)?;
    Ok(row.as_ref().map(run_of))
}

/// The machine of the box the run works in, if it has one.
pub async fn run_box(db: &Db, team: Uuid, run: Uuid) -> Result<Option<String>, String> {
    let row = db.client().await?.query_opt("select box from team_runs where team = $1 and id = $2", &[&team, &run]).await.map_err(err)?;
    Ok(row.and_then(|r| r.get(0)))
}

/// The ids of the team's live runs of bot `workflow`.
pub async fn live_runs_of(db: &Db, team: Uuid, workflow: &str) -> Result<Vec<Uuid>, String> {
    let rows = db
        .client()
        .await?
        .query(&format!("select id from team_runs where team = $1 and workflow = $2 and state in {LIVE}"), &[&team, &workflow])
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// The team's live run whose session runs on `machine`, if any.
pub async fn run_on_machine(db: &Db, team: Uuid, machine: &str) -> Result<Option<Run>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!("select {RUN_COLUMNS} from team_runs where team = $1 and box = $2 and state in {LIVE} order by started_at desc limit 1"),
            &[&team, &machine],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(run_of))
}

/// The workflow and subject of each live run of the team bound to `machine` (its `box`), which
/// a team box's session callbacks are held to. A run with no subject recorded has None.
pub async fn bound_runs(db: &Db, team: Uuid, machine: &str) -> Result<Vec<(String, Option<Subject>)>, String> {
    let rows = db
        .client()
        .await?
        .query(&format!("select workflow, subject from team_runs where team = $1 and box = $2 and state in {LIVE}"), &[&team, &machine])
        .await
        .map_err(err)?;
    Ok(rows
        .iter()
        .map(|r| (r.get(0), r.get::<_, Option<serde_json::Value>>(1).and_then(|subject| serde_json::from_value(subject).ok())))
        .collect())
}

/// Moves a run that has not ended to `state` with `controller`; sets `ended_at` and `outcome`
/// when `state` ends it. False when the run had already ended.
pub async fn set_run_state(db: &Db, team: Uuid, run: Uuid, state: RunState, controller: Option<Uuid>, outcome: Option<&str>) -> Result<bool, String> {
    let ends = state.ended();
    let moved = db
        .client()
        .await?
        .execute(
            &format!(
                "update team_runs set state = $3, controller = $4, ended_at = case when $5 then now() else ended_at end,
                   outcome = case when $5 then $6 else outcome end,
                   waiting_secs = case when $3 = 'waiting' then waiting_secs else {} end
                 where team = $1 and id = $2 and state in {LIVE}",
                waited!()
            ),
            &[&team, &run, &state.as_str(), &controller, &ends, &outcome],
        )
        .await
        .map_err(err)?;
    Ok(moved == 1)
}

/// Moves a run that works (`working` or `taken_over`) to `waiting`, in one statement, so of two
/// asks at once only one gets through. None when it was not working.
pub async fn start_waiting(db: &Db, team: Uuid, run: Uuid) -> Result<Option<Run>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!("update team_runs set state = 'waiting', waiting_since = now() where team = $1 and id = $2 and state in ('working', 'taken_over') returning {RUN_COLUMNS}"),
            &[&team, &run],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(run_of))
}

/// Moves a waiting run back to work: `taken_over` while someone controls it, else `working`.
/// None when it was not waiting.
pub async fn stop_waiting(db: &Db, team: Uuid, run: Uuid) -> Result<Option<Run>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!(
                "update team_runs set state = case when controller is null then 'working' else 'taken_over' end, waiting_secs = {}
                 where team = $1 and id = $2 and state = 'waiting' returning {RUN_COLUMNS}",
                waited!()
            ),
            &[&team, &run],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(run_of))
}

/// Makes `member` the controller of a live run nobody controls: `taken_over`, or still `waiting`
/// while its call is open (the answer moves it on). None when someone controls it or it ended.
pub async fn take_over(db: &Db, team: Uuid, run: Uuid, member: Uuid) -> Result<Option<Run>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!(
                "update team_runs set controller = $3, state = case when state = 'waiting' then 'waiting' else 'taken_over' end
                 where team = $1 and id = $2 and controller is null and state in ('working', 'waiting') returning {RUN_COLUMNS}"
            ),
            &[&team, &run, &member],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(run_of))
}

/// Clears the controller of a live run: `working` again, or still `waiting` while its call is
/// open. None when nobody controlled it or it ended.
pub async fn give_back(db: &Db, team: Uuid, run: Uuid) -> Result<Option<Run>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!(
                "update team_runs set controller = null, state = case when state = 'waiting' then 'waiting' else 'working' end
                 where team = $1 and id = $2 and controller is not null and state in {LIVE} returning {RUN_COLUMNS}"
            ),
            &[&team, &run],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(run_of))
}

/// The live runs `member` controls.
pub async fn runs_controlled_by(db: &Db, team: Uuid, member: Uuid) -> Result<Vec<Uuid>, String> {
    let rows = db
        .client()
        .await?
        .query(&format!("select id from team_runs where team = $1 and controller = $2 and state in {LIVE}"), &[&team, &member])
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Whether `who` takes in any member of the team now: a member named, a group with a member, or a
/// group with none whose fallback is a member; the owner always.
pub async fn reaches_anyone(db: &Db, team: Uuid, who: &Who) -> Result<bool, String> {
    let client = db.client().await?;
    let row = match who {
        Who::Owner => return Ok(true),
        Who::Person { account } => {
            let Ok(account) = Uuid::parse_str(account) else { return Ok(false) };
            client.query_one("select exists (select 1 from team_members where team = $1 and member = $2)", &[&team, &account]).await
        }
        Who::Group { handle } => {
            client
                .query_one(
                    "select exists (select 1 from team_group_members where team = $1 and handle = $2)
                         or exists (select 1 from team_groups g join team_members m on m.team = g.team and m.member = g.fallback
                                      where g.team = $1 and g.handle = $2)",
                    &[&team, &handle],
                )
                .await
        }
    };
    Ok(row.map_err(err)?.get(0))
}

/// Who the bot of `workflow` asks, and the member who answers for it; None when it has no row.
pub async fn bot_calls(db: &Db, team: Uuid, workflow: &str) -> Result<Option<(CallRules, Option<Uuid>)>, String> {
    let row = db.client().await?.query_opt("select calls, owner from team_bots where team = $1 and workflow = $2", &[&team, &workflow]).await.map_err(err)?;
    let Some(row) = row else { return Ok(None) };
    let rules = serde_json::from_value(row.get(0)).map_err(|e| format!("the bot's call rules do not read: {e}"))?;
    Ok(Some((rules, row.get(1))))
}

/// Whether `member` is in the group `handle` now, or is its fallback while no member of the
/// team is in it.
pub async fn asks_member(db: &Db, team: Uuid, handle: &str, member: Uuid) -> Result<bool, String> {
    let row = db
        .client()
        .await?
        .query_one(
            "select exists (select 1 from team_group_members where team = $1 and handle = $2 and member = $3)
                 or exists (select 1 from team_groups g where g.team = $1 and g.handle = $2 and g.fallback = $3
                              and exists (select 1 from team_members m where m.team = $1 and m.member = $3)
                              and not exists (select 1 from team_group_members gm where gm.team = $1 and gm.handle = $2))",
            &[&team, &handle, &member],
        )
        .await
        .map_err(err)?;
    Ok(row.get(0))
}

pub async fn group_exists(db: &Db, team: Uuid, handle: &str) -> Result<bool, String> {
    let row = db.client().await?.query_one("select exists (select 1 from team_groups where team = $1 and handle = $2)", &[&team, &handle]).await.map_err(err)?;
    Ok(row.get(0))
}

/// The Slack user-group handle of group `handle`, empty when it has none or the team has no such group.
pub async fn slack_group(db: &Db, team: Uuid, handle: &str) -> Result<String, String> {
    let row = db.client().await?.query_opt("select slack_group from team_groups where team = $1 and handle = $2", &[&team, &handle]).await.map_err(err)?;
    Ok(row.map(|r| r.get(0)).unwrap_or_default())
}

/// The account's email, for its name (`calls::name_of`).
pub async fn email(db: &Db, account: Uuid) -> Result<Option<String>, String> {
    let row = db.client().await?.query_opt("select email from accounts where id = $1", &[&account]).await.map_err(err)?;
    Ok(row.map(|r| r.get(0)))
}

/// Opens a call on its run, created now by Postgres's clock; answers it as kept. `step` is the
/// hash of the exact step the team-box hook asks about.
pub async fn insert_call(db: &Db, team: Uuid, call: &Call, step: Option<&str>) -> Result<Call, String> {
    let (id, run) = (Uuid::parse_str(&call.id).map_err(|_| "a call's id is a uuid")?, Uuid::parse_str(&call.run).map_err(|_| "a run's id is a uuid")?);
    let row = db
        .client()
        .await?
        .query_one(
            &format!(
                "with c as (insert into team_calls (id, team, run, situation, asked, question, choices, step_hash) values ($1, $2, $3, $4, $5, $6, $7, $8) returning *)
                 select {CALL_COLUMNS} from c join team_runs r on r.id = c.run"
            ),
            &[&id, &team, &run, &call.situation.as_str(), &json(&call.asked)?, &call.question, &json(&call.choices)?, &step],
        )
        .await
        .map_err(err)?;
    Ok(call_of(&row)?.call)
}

/// Opens `call`, passed on from call `first`, keeping `first`'s `created_at`, so passing restarts
/// neither the escalation nor the give-up clock. Only while the run still waits, in one statement:
/// None when it ended meanwhile, as when someone stopped it between the pass and this.
pub async fn pass_call(db: &Db, team: Uuid, call: &Call, step: Option<&str>, first: Uuid) -> Result<Option<Call>, String> {
    let (id, run) = (Uuid::parse_str(&call.id).map_err(|_| "a call's id is a uuid")?, Uuid::parse_str(&call.run).map_err(|_| "a run's id is a uuid")?);
    let row = db
        .client()
        .await?
        .query_opt(
            &format!(
                "with c as (insert into team_calls (id, team, run, situation, asked, question, choices, step_hash, created_at)
                   select $1, $2, $3, $4, $5, $6, $7, $8, f.created_at from team_runs w, team_calls f
                   where w.team = $2 and w.id = $3 and w.state = 'waiting' and f.team = $2 and f.id = $9
                   returning *)
                 select {CALL_COLUMNS} from c join team_runs r on r.id = c.run"
            ),
            &[&id, &team, &run, &call.situation.as_str(), &json(&call.asked)?, &call.question, &json(&call.choices)?, &step, &first],
        )
        .await
        .map_err(err)?;
    row.as_ref().map(call_of).transpose().map(|row| row.map(|row| row.call))
}

/// Uses the "Go ahead" a person gave on a `before_irreversible` call of run `run` about the step
/// hashed `step` and shown as `question`: the call's id, once, and only within 10 minutes of the
/// answer's `at` (`now` is unix seconds). One statement, so of two asks at once only one uses it.
pub async fn use_go_ahead(db: &Db, team: Uuid, run: Uuid, step: &str, question: &str, now: i64) -> Result<Option<Uuid>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            "update team_calls set released_at = $5::bigint where id = (
               select id from team_calls
               where team = $1 and run = $2 and step_hash = $3 and question = $4 and situation = 'before_irreversible'
                 and answer -> 'kind' ->> 'kind' = 'choice' and answer ->> 'choice' = $6
                 and released_at is null and (answer ->> 'at')::bigint >= $5::bigint - 600
               order by answered_at desc limit 1 for update)
             returning id",
            &[&team, &run, &step, &question, &now, &GO_AHEAD],
        )
        .await
        .map_err(err)?;
    Ok(row.map(|r| r.get(0)))
}

/// Spends every unused "Go ahead" of run `run` (`now` is unix seconds): a take-over hands the run
/// to someone else, and a step the bot asked about before is asked about again.
pub async fn release_go_aheads(db: &Db, team: Uuid, run: Uuid, now: i64) -> Result<u64, String> {
    db.client()
        .await?
        .execute(
            "update team_calls set released_at = $3::bigint
             where team = $1 and run = $2 and situation = 'before_irreversible' and released_at is null
               and answer -> 'kind' ->> 'kind' = 'choice' and answer ->> 'choice' = $4",
            &[&team, &run, &now, &GO_AHEAD],
        )
        .await
        .map_err(err)
}

pub async fn call(db: &Db, team: Uuid, call: Uuid) -> Result<Option<CallRow>, String> {
    let row = db
        .client()
        .await?
        .query_opt(&format!("select {CALL_COLUMNS} from team_calls c join team_runs r on r.id = c.run where c.team = $1 and c.id = $2"), &[&team, &call])
        .await
        .map_err(err)?;
    row.as_ref().map(call_of).transpose()
}

/// The run's open call, if any.
pub async fn open_call(db: &Db, team: Uuid, run: Uuid) -> Result<Option<Call>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!("select {CALL_COLUMNS} from team_calls c join team_runs r on r.id = c.run where c.team = $1 and c.run = $2 and c.answered_at is null limit 1"),
            &[&team, &run],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(call_of).transpose()?.map(|row| row.call))
}

/// The open calls, then the last 20 answered, newest first.
pub async fn calls(db: &Db, team: Uuid) -> Result<Vec<Call>, String> {
    let rows = db
        .client()
        .await?
        .query(
            &format!(
                "(select {CALL_COLUMNS}, 0 as part, c.created_at as at from team_calls c join team_runs r on r.id = c.run where c.team = $1 and c.answered_at is null)
                 union all
                 (select {CALL_COLUMNS}, 1, c.answered_at from team_calls c join team_runs r on r.id = c.run where c.team = $1 and c.answered_at is not null
                  order by c.answered_at desc limit 20)
                 order by part, at desc"
            ),
            &[&team],
        )
        .await
        .map_err(err)?;
    Ok(calls_of(&rows)?.into_iter().map(|row| row.call).collect())
}

/// Answers a call nobody answered yet, in one statement guarded by `answered_at is null`, so of
/// two answers (or an answer and the give-up) only one wins. None when someone already did.
pub async fn answer_call(db: &Db, team: Uuid, call: Uuid, answer: &Answer, outcome: &str) -> Result<Option<Call>, String> {
    let by = Uuid::parse_str(&answer.by).map_err(|_| "an answer's author is an account id")?;
    let row = db
        .client()
        .await?
        .query_opt(
            &format!(
                "update team_calls c set answered_by = $3, answered_at = now(), answer = $4, outcome = $5
                 from team_runs r where r.id = c.run and c.team = $1 and c.id = $2 and c.answered_at is null
                 returning {CALL_COLUMNS}"
            ),
            &[&team, &call, &by, &json(answer)?, &outcome],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(call_of).transpose()?.map(|row| row.call))
}

/// The open call `call`, not escalated yet, now escalated as `escalate_due` does it: `escalated_at`
/// set and the owner added to `asked`. None when it was answered or escalated already.
pub async fn escalate_now(db: &Db, team: Uuid, call: Uuid) -> Result<Option<Call>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!(
                "update team_calls c set escalated_at = now(),
                   asked = case when c.asked @> '[{{\"kind\":\"owner\"}}]'::jsonb then c.asked else c.asked || '[{{\"kind\":\"owner\"}}]'::jsonb end
                 from team_runs r
                 where r.id = c.run and c.team = $1 and c.id = $2 and c.answered_at is null and c.escalated_at is null
                 returning {CALL_COLUMNS}"
            ),
            &[&team, &call],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(call_of).transpose()?.map(|row| row.call))
}

/// Calls nobody answered within `secs` and not escalated yet, now escalated: `escalated_at` set
/// and the owner added to `asked`. One statement, for `tick`.
pub async fn escalate_due(db: &Db, secs: u64) -> Result<Vec<CallRow>, String> {
    let rows = db
        .client()
        .await?
        .query(
            &format!(
                "update team_calls c set escalated_at = now(),
                   asked = case when c.asked @> '[{{\"kind\":\"owner\"}}]'::jsonb then c.asked else c.asked || '[{{\"kind\":\"owner\"}}]'::jsonb end
                 from team_runs r
                 where r.id = c.run and c.answered_at is null and c.escalated_at is null and c.created_at <= now() - make_interval(secs => $1)
                 returning {CALL_COLUMNS}"
            ),
            &[&(secs as f64)],
        )
        .await
        .map_err(err)?;
    calls_of(&rows)
}

/// Calls nobody answered within `secs`, now closed as `gave-up`, guarded by `answered_at is
/// null` as an answer is. One statement, for `tick`.
pub async fn give_up_due(db: &Db, secs: u64) -> Result<Vec<CallRow>, String> {
    let rows = db
        .client()
        .await?
        .query(
            &format!(
                "update team_calls c set answered_at = now(), outcome = 'gave-up'
                 from team_runs r
                 where r.id = c.run and c.answered_at is null and c.created_at <= now() - make_interval(secs => $1)
                 returning {CALL_COLUMNS}"
            ),
            &[&(secs as f64)],
        )
        .await
        .map_err(err)?;
    calls_of(&rows)
}

/// Closes the run's open call as `stopped` with `answer`, when it has one.
pub async fn stop_open_call(db: &Db, team: Uuid, run: Uuid, answer: &Answer) -> Result<Option<Call>, String> {
    let by = Uuid::parse_str(&answer.by).map_err(|_| "an answer's author is an account id")?;
    let row = db
        .client()
        .await?
        .query_opt(
            &format!(
                "update team_calls c set answered_by = $3, answered_at = now(), answer = $4, outcome = 'stopped'
                 from team_runs r where r.id = c.run and c.team = $1 and c.run = $2 and c.answered_at is null
                 returning {CALL_COLUMNS}"
            ),
            &[&team, &run, &by, &json(answer)?],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(call_of).transpose()?.map(|row| row.call))
}

/// Sets `machine` on the live run of the team's workflow `workflow` opened for `subject` that has
/// no box yet, just before the box's session starts (`boxes::Boxes`), so a session that asks
/// people at once finds its run. Answers whether a live run has the box now: false for a person's
/// account, and for a team's run that ended while its box booted.
pub async fn claim_run_box(db: &Db, account: Uuid, workflow: &str, subject: &Subject, machine: &str) -> Result<bool, String> {
    let claimed = db
        .client()
        .await?
        .execute(
            &format!("update team_runs set box = $4 where team = $1 and workflow = $2 and subject = $3 and (box is null or box = $4) and state in {LIVE}"),
            &[&account, &workflow, &json(subject)?, &machine],
        )
        .await
        .map_err(err)?;
    Ok(claimed > 0)
}

/// The machines of the account's boxes whose run waits on an open call, which the box reaper
/// does not end for idleness (`boxes::Boxes::reap`). Empty for a person's account.
pub async fn waiting_boxes(db: &Db, account: Uuid) -> Result<std::collections::HashSet<String>, String> {
    let rows = db
        .client()
        .await?
        .query(
            "select distinct r.box from team_runs r join team_calls c on c.run = r.id
             where r.team = $1 and r.box is not null and c.answered_at is null",
            &[&account],
        )
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// The running boxes of team accounts that no live run holds, as (team, machine): no live run has
/// the box's machine, and none still without a box was opened for the box's workflow and subject
/// (a run's box is set only once its session starts). For `calls::tick`.
pub async fn unheld_team_boxes(db: &Db) -> Result<Vec<(Uuid, Uuid)>, String> {
    let rows = db
        .client()
        .await?
        .query(
            &format!(
                "select b.account, b.machine from boxes b join accounts a on a.id = b.account and a.kind = 'team'
                 where b.phase = 'running' and not exists (
                   select 1 from team_runs r where r.team = b.account and r.state in {LIVE}
                     and (r.box = b.machine::text or (r.box is null and r.workflow = b.connector and r.subject = b.subject)))"
            ),
            &[],
        )
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// Live runs whose box is not there, as (team, run, never started): one with no box that started
/// more than `secs` ago (true), and one whose box has no `boxes` row (false). A reserved run is
/// left to `stale_reservations`, its own bound. For `calls::tick`.
pub async fn boxless_runs(db: &Db, secs: u64) -> Result<Vec<(Uuid, Uuid, bool)>, String> {
    let rows = db
        .client()
        .await?
        .query(
            &format!(
                "select r.team, r.id, r.box is null from team_runs r
                 where r.state in {LIVE} and not r.reserved and case when r.box is null then r.started_at <= now() - make_interval(secs => $1)
                   else not exists (select 1 from boxes b where b.account = r.team and b.machine::text = r.box) end"
            ),
            &[&(secs as f64)],
        )
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}

/// Live reserved runs whose place was held more than `secs` ago and whose session never opened,
/// as (team, run, workflow). For `calls::tick`.
pub async fn stale_reservations(db: &Db, secs: u64) -> Result<Vec<(Uuid, Uuid, String)>, String> {
    let rows = db
        .client()
        .await?
        .query(
            &format!("select team, id, workflow from team_runs where state in {LIVE} and reserved and reserved_at <= now() - make_interval(secs => $1)"),
            &[&(secs as f64)],
        )
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}

/// Ends reserved run `run` as `state` with `outcome`, like `set_run_state`, only while it is live,
/// still reserved and its place was held at least `secs` ago, in one statement: the backstop's
/// end of a reservation that never opened. False when any of that no longer holds.
///
/// The claim (`claim`) writes the same row while the run is live and reserved, so exactly one of
/// them wins: Postgres's row lock and its re-check of the row under READ COMMITTED decide.
/// - This write locks the row first: the run ends; the claim's update waits, re-checks, finds it
///   ended, and takes nothing.
/// - The claim locks the row first: it holds the place anew (`reserved_at = now()`); this write
///   waits, re-checks, finds the place no longer past the limit, and answers false.
pub async fn end_stale_reservation(db: &Db, team: Uuid, run: Uuid, state: RunState, controller: Option<Uuid>, outcome: &str, secs: u64) -> Result<bool, String> {
    let ended = db
        .client()
        .await?
        .query_opt(
            &format!(
                "update team_runs set state = $3, controller = $4, ended_at = now(), outcome = $5
                 where team = $1 and id = $2 and state in {LIVE} and reserved and reserved_at <= now() - make_interval(secs => $6)
                 returning id"
            ),
            &[&team, &run, &state.as_str(), &controller, &outcome, &(secs as f64)],
        )
        .await
        .map_err(err)?;
    Ok(ended.is_some())
}

/// Takes out of its line the granted event that reserved run `run` holds the place of, if no
/// poller claimed it yet, so it never runs without its place.
pub async fn drop_grant(db: &Db, team: Uuid, run: Uuid) -> Result<(), String> {
    db.client().await?.execute("delete from team_run_queue where team = $1 and run = $2 and granted_at is not null", &[&team, &run]).await.map_err(err).map(drop)
}

/// Runs left waiting with no open call for more than `secs`, none of whose calls closed within
/// `secs` either, as (team, run, whether its box runs). An ask opens its call, and an answer moves
/// its run on, within moments, so such a run was left by a give-up or a restart. For `calls::tick`.
pub async fn stale_waiting(db: &Db, secs: u64) -> Result<Vec<(Uuid, Uuid, bool)>, String> {
    let rows = db
        .client()
        .await?
        .query(
            "select r.team, r.id, exists (select 1 from boxes b where b.account = r.team and b.machine::text = r.box and b.phase <> 'ending')
             from team_runs r
             where r.state = 'waiting' and coalesce(r.waiting_since, r.started_at) <= now() - make_interval(secs => $1)
               and not exists (select 1 from team_calls c where c.run = r.id
                 and (c.answered_at is null or c.answered_at > now() - make_interval(secs => $1)))",
            &[&(secs as f64)],
        )
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}

// ---- triggers.rs ----

/// The live run of the team's workflow `workflow` opened for `subject`, if any.
pub async fn live_run_for(db: &Db, team: Uuid, workflow: &str, subject: &Subject) -> Result<Option<Run>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!("select {RUN_COLUMNS} from team_runs where team = $1 and workflow = $2 and subject = $3 and state in {LIVE} order by started_at desc limit 1"),
            &[&team, &workflow, &json(subject)?],
        )
        .await
        .map_err(err)?;
    Ok(row.as_ref().map(run_of))
}

/// An event waiting in its bot's line (`team_run_queue`), oldest first by `id`.
pub struct Waiting {
    pub id: i64,
    pub workflow: String,
    /// What `teams::queue` stored to start it with.
    pub start: serde_json::Value,
}

/// How many runs of the team are live.
pub async fn live_count(db: &Db, team: Uuid) -> Result<i64, String> {
    live_count_on(&*db.client().await?, team).await
}

/// `live_count` on `client`: in a team's turn, its own reservations count.
pub async fn live_count_on(client: &Client, team: Uuid) -> Result<i64, String> {
    let row = client.query_one(&format!("select count(*) from team_runs where team = $1 and state in {LIVE}"), &[&team]).await.map_err(err)?;
    Ok(row.get(0))
}

/// The most runs the team may have live at once.
pub async fn max_live_runs(db: &Db, team: Uuid) -> Result<i64, String> {
    let row = db.client().await?.query_opt("select max_live_runs from teams where id = $1", &[&team]).await.map_err(err)?;
    Ok(row.map_or(3, |r| i64::from(r.get::<_, i32>(0))))
}

/// How many events wait in the line of bot `workflow`.
pub async fn waiting_count(db: &Db, team: Uuid, workflow: &str) -> Result<i64, String> {
    let row = db.client().await?.query_one("select count(*) from team_run_queue where team = $1 and workflow = $2 and granted_at is null", &[&team, &workflow]).await.map_err(err)?;
    Ok(row.get(0))
}

/// Puts `start` at the end of the line of bot `workflow`, and drops its oldest beyond `most`.
/// Answers how many it dropped.
pub async fn enqueue(db: &Db, team: Uuid, workflow: &str, start: &serde_json::Value, most: i64) -> Result<u64, String> {
    let client = db.client().await?;
    client.execute("insert into team_run_queue (team, workflow, start) values ($1, $2, $3)", &[&team, &workflow, start]).await.map_err(err)?;
    client
        .execute(
            "delete from team_run_queue where id in (select id from team_run_queue where team = $1 and workflow = $2 and granted_at is null order by id desc offset $3)",
            &[&team, &workflow, &most],
        )
        .await
        .map_err(err)
}

/// The oldest waiting event, not granted yet, of a bot of the team that has no live run, `first`'s
/// before any other bot's. Read on `held`, the connection of the team's turn.
pub async fn next_waiting(held: &Client, team: Uuid, first: &str) -> Result<Option<Waiting>, String> {
    let row = held
        .query_opt(
            &format!(
                "select id, workflow, start from team_run_queue q where team = $1 and granted_at is null
                 and not exists (select 1 from team_runs r where r.team = q.team and r.workflow = q.workflow and r.state in {LIVE})
                 order by (workflow = $2) desc, id limit 1"
            ),
            &[&team, &first],
        )
        .await
        .map_err(err)?;
    Ok(row.map(|r| Waiting { id: r.get(0), workflow: r.get(1), start: r.get(2) }))
}

/// The waiting event of bot `workflow` on `subject` (`queue::Stored::Event`), if one waits: one
/// granted its place and not claimed yet waits too.
pub async fn waiting_event_for(db: &Db, team: Uuid, workflow: &str, subject: &Subject) -> Result<Option<Waiting>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            "select id, workflow, start from team_run_queue where team = $1 and workflow = $2 and start->>'by' = 'event' and start->'event'->'subject' = $3 order by id limit 1",
            &[&team, &workflow, &json(subject)?],
        )
        .await
        .map_err(err)?;
    Ok(row.map(|r| Waiting { id: r.get(0), workflow: r.get(1), start: r.get(2) }))
}

/// Stores what waiting event `id` starts with: false when its row is gone, as when a poller
/// claimed it meanwhile.
pub async fn set_waiting(db: &Db, id: i64, start: &serde_json::Value) -> Result<bool, String> {
    db.client().await?.execute("update team_run_queue set start = $2 where id = $1", &[&id, start]).await.map_err(err).map(|n| n == 1)
}

/// Waiting event `id`'s turn came: it stays in its line, granted the place reserved run `run`
/// holds, until its team's poller claims it (`claim`). Written on `held`, in the team's turn,
/// with the reserved run's insert, so a hub that stops keeps both or neither.
pub async fn grant(held: &Client, id: i64, run: Uuid) -> Result<(), String> {
    held.execute("update team_run_queue set granted_at = now(), run = $2 where id = $1", &[&id, &run]).await.map_err(err).map(drop)
}

/// Each team that has a free place and an event, not granted yet, waiting for a bot with no live
/// run: the team and that oldest event's bot. The reaper starts them (`calls::reap`).
pub async fn startable_waiting(db: &Db) -> Result<Vec<(Uuid, String)>, String> {
    let rows = db
        .client()
        .await?
        .query(
            &format!(
                "select distinct on (q.team) q.team, q.workflow from team_run_queue q join teams t on t.id = q.team
                 where q.granted_at is null
                   and not exists (select 1 from team_runs r where r.team = q.team and r.workflow = q.workflow and r.state in {LIVE})
                   and (select count(*) from team_runs r where r.team = q.team and r.state in {LIVE}) < t.max_live_runs
                 order by q.team, q.id"
            ),
            &[],
        )
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// The team's granted events (`grant`) that no poller claimed yet, oldest first.
pub async fn granted(db: &Db, team: Uuid) -> Result<Vec<i64>, String> {
    let rows = db.client().await?.query("select id from team_run_queue where team = $1 and granted_at is not null order by id", &[&team]).await.map_err(err)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Claims granted event `id` for the poller that runs it: only the claim that deletes the row
/// gets its stored event and the reserved run that holds its place, so of two hubs exactly one runs it.
/// Only while that run is still live and reserved, and in the same statement the claim holds that
/// place anew (`reserved_at = now()`), so the claimed run gets a whole limit to open its session.
///
/// The claim and the backstop (`end_stale_reservation`) both write the run's row, each only while
/// it is live and reserved, so exactly one of them wins, and the claim has no limit of its own:
/// Postgres's row lock and its re-check of the row under READ COMMITTED decide.
/// - The claim locks the row first: it holds the place anew and takes the granted row; the
///   backstop's write waits, re-checks the row, finds the place no longer past the limit, and
///   ends nothing. The run starts once.
/// - The backstop locks the row first: it ends the run; the claim's update waits, re-checks the
///   row, finds the run ended, and takes nothing. The run is ended and counted once.
/// The granted row is locked first (`for update`), so a claim that holds the place anew always
/// takes its row: a granted row someone else took meanwhile leaves the run as it was. A granted
/// row another claim has locked is skipped (`skip locked`): that claim runs it, and this one gets
/// nothing, without waiting on the shared connection.
pub async fn claim(db: &Db, team: Uuid, id: i64) -> Result<Option<(serde_json::Value, Option<Uuid>)>, String> {
    let row = db
        .client()
        .await?
        .query_opt(
            &format!(
                "with granted as (select run from team_run_queue where team = $1 and id = $2 and granted_at is not null for update skip locked),
                      held as (update team_runs r set reserved_at = now() from granted
                               where r.team = $1 and r.id = granted.run and r.reserved and r.state in {LIVE} returning r.id)
                 delete from team_run_queue q using held where q.team = $1 and q.id = $2 and q.run = held.id
                 returning q.start, q.run"
            ),
            &[&team, &id],
        )
        .await
        .map_err(err)?;
    Ok(row.map(|r| (r.get(0), r.get(1))))
}

/// Takes waiting event `id` out of its line. Written on `held`, the connection of the team's
/// turn, so it commits with the turn: a turn that fails after it leaves the event waiting.
pub async fn take_waiting(held: &Client, id: i64) -> Result<(), String> {
    held.execute("delete from team_run_queue where id = $1", &[&id]).await.map_err(err).map(drop)
}

/// Counts `count` more events of bot `workflow` that started no run for `reason`.
pub async fn count_skipped(db: &Db, team: Uuid, workflow: &str, reason: SkipReason, count: u64) -> Result<(), String> {
    count_skipped_on(&*db.client().await?, team, workflow, reason, count).await
}

/// `count_skipped` on `client`: a team's turn counts on its own connection, so a count commits
/// with the take it goes with.
pub async fn count_skipped_on(client: &Client, team: Uuid, workflow: &str, reason: SkipReason, count: u64) -> Result<(), String> {
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    client
        .execute(
            "insert into team_bot_skips (team, workflow, reason, count) values ($1, $2, $3, $4)
             on conflict (team, workflow, reason) do update set count = team_bot_skips.count + excluded.count",
            &[&team, &workflow, &reason.as_str(), &count],
        )
        .await
        .map_err(err)
        .map(drop)
}

/// The skipped events of bot `workflow`, by reason.
pub async fn skipped(db: &Db, team: Uuid, workflow: &str) -> Result<Vec<Skipped>, String> {
    let rows = db.client().await?.query("select reason, count from team_bot_skips where team = $1 and workflow = $2 order by reason", &[&team, &workflow]).await.map_err(err)?;
    Ok(rows
        .iter()
        .filter_map(|r| SkipReason::parse(r.get(0)).map(|reason| Skipped { reason, count: u64::try_from(r.get::<_, i64>(1)).unwrap_or(0) }))
        .collect())
}

// ---- limits (calls::tick) ----

/// Charges each run at work (`working` or `taken_over`) in a box for its active time at its box's
/// hourly rate (`small`, `medium`, `large` in `rates`): its `cost_usd` never goes down, and a
/// waiting run is not charged, so a wait costs nothing toward its `BotLimits::usd`.
pub async fn charge_working_runs(db: &Db, rates: [f64; 3]) -> Result<(), String> {
    db.client()
        .await?
        .execute(
            "update team_runs r set cost_usd = greatest(r.cost_usd,
                   greatest(0, extract(epoch from now() - coalesce(r.opened_at, r.started_at))::bigint - r.waiting_secs)::float8
                   * case lower(b.size) when 'medium' then $2::float8 when 'large' then $3::float8 else $1::float8 end / 3600.0)
                 from boxes b where b.account = r.team and b.machine::text = r.box and r.state in ('working', 'taken_over') and not r.reserved",
            &[&rates[0], &rates[1], &rates[2]],
        )
        .await
        .map_err(err)?;
    Ok(())
}

/// A run at work that reached a limit of its bot and was not called for it yet.
pub struct AtLimit {
    pub team: Uuid,
    pub run: Uuid,
    pub limits: BotLimits,
    /// True for its dollars, false for its minutes.
    pub spent: bool,
}

/// The runs at work (not reserved) whose active time reached their bot's `minutes`, or whose
/// `cost_usd` its `usd`, and that have no `on_limit` call yet: one limit call per run.
pub async fn runs_at_limit(db: &Db) -> Result<Vec<AtLimit>, String> {
    let rows = db
        .client()
        .await?
        .query(
            "select r.team, r.id, b.limits, coalesce(r.cost_usd >= (b.limits->>'usd')::float8, false), b.workflow
                 from team_runs r join team_bots b on b.team = r.team and b.workflow = r.workflow
                 where r.state in ('working', 'taken_over') and not r.reserved
                   and (r.cost_usd >= (b.limits->>'usd')::float8
                        or extract(epoch from now() - coalesce(r.opened_at, r.started_at))::bigint - r.waiting_secs >= (b.limits->>'minutes')::bigint * 60)
                   and not exists (select 1 from team_calls c where c.run = r.id and c.situation = 'on_limit')",
            &[],
        )
        .await
        .map_err(err)?;
    let mut found = Vec::with_capacity(rows.len());
    for r in &rows {
        // One bot's unreadable row holds up no other run's limit: it is skipped, and logged.
        let Ok(limits) = serde_json::from_value(r.get(2)) else {
            let (team, workflow): (Uuid, String) = (r.get(0), r.get(4));
            eprintln!("pinned-server: the limits of bot {workflow} of team {team} are not readable: its runs are not held to them");
            continue;
        };
        found.push(AtLimit { team: r.get(0), run: r.get(1), limits, spent: r.get(3) });
    }
    Ok(found)
}

/// The live runs started at least `secs` ago, waits included (`TeamPolicy::max_life_secs`).
pub async fn runs_past_max_life(db: &Db, secs: u64) -> Result<Vec<(Uuid, Uuid)>, String> {
    let rows = db
        .client()
        .await?
        .query(&format!("select team, id from team_runs where state in {LIVE} and started_at <= now() - make_interval(secs => $1)"), &[&(secs as f64)])
        .await
        .map_err(err)?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}
