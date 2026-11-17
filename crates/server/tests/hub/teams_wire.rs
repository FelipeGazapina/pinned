//! A team's open invites as the owner lists them, and a bot's save that answers its workflow and
//! can be sent again without making a second workflow.

use super::teams_membership::{db_text, join_team, make_team, new_invite, person, team_op};
use super::*;
use uuid::Uuid;

const NOT_OWNER: &str = "only the team's owner does this";
const BUSY: &str = "the hub is busy with this right now; try again in a moment";

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

/// A workflow a member saves with its bot, as the app sends it; `id` empty for a new one the hub
/// names, as an older app sends it.
fn workflow(id: &str) -> Value {
    json!({ "id": id, "name": "Fix bugs", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
        "folder": "/w/team", "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false, "instructions": "Fix the bug." })
}

fn save_bot(id: &str) -> Value {
    json!({ "op": "save_bot", "bot": { "workflow": id }, "connector": workflow(id) })
}

/// A second connection to the hub's database that holds `team_bots` against every write, but not
/// against reads, until it is dropped.
struct HeldBots {
    runtime: tokio::runtime::Runtime,
    client: tokio_postgres::Client,
}

impl HeldBots {
    fn hold(database: &str) -> HeldBots {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let client = runtime.block_on(async {
            let (client, connection) = tokio_postgres::connect(&format!("{}/{database}", postgres()), tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(connection);
            client.batch_execute("begin; lock table team_bots in share row exclusive mode").await.unwrap();
            client
        });
        HeldBots { runtime, client }
    }

    fn release(self) {
        self.runtime.block_on(self.client.batch_execute("rollback")).unwrap();
    }
}

#[test]
fn the_owner_lists_the_open_invites_only_and_a_member_hears_the_refusal() {
    let hub = TestHub::start("teams_wire_invites");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let (mut bruno, _) = person(&url, "bruno@example.test");
    let team = make_team(&mut ana, "Acme");

    let kept = new_invite(&mut ana, &team);
    let revoked = new_invite(&mut ana, &team);
    let used = new_invite(&mut ana, &team);
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "revoke_invite", "hint": &revoked[..4] })), Ok(Value::Null));
    join_team(&mut bruno, &used).unwrap();

    let open = team_op(&mut ana, &team, json!({ "op": "invites" })).unwrap();
    let rows = open.as_array().unwrap_or_else(|| panic!("a list: {open}"));
    assert_eq!(rows.len(), 1, "only the open invite: {open}");
    let created = rows[0]["created"].as_i64().unwrap();
    assert!((now_secs() - 60..=now_secs()).contains(&created), "made within the last minute: {created}");
    assert_eq!(rows[0], json!({ "hint": &kept[..4], "created": created, "made_by": "ana" }), "never the code");

    // Newest first.
    let newer = new_invite(&mut ana, &team);
    let open = team_op(&mut ana, &team, json!({ "op": "invites" })).unwrap();
    let hints: Vec<&str> = open.as_array().unwrap().iter().map(|r| r["hint"].as_str().unwrap()).collect();
    assert_eq!(hints, vec![&newer[..4], &kept[..4]]);

    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "invites" })), Err(NOT_OWNER.into()));
}

const NO_INVITE: &str = "that invite code is unknown, used or revoked";
const NOT_A_MEMBER: &str = "you are not a member of that team";
const FULL: &str = "this team has 20 open invite codes; revoke some first";

/// The hub has no call that deletes an account, so the joiner's account goes by `delete from
/// accounts` on the hub's database, as the account's own cascade would remove it.
#[test]
fn a_used_invite_stays_used_when_its_joiner_is_deleted() {
    let hub = TestHub::start("teams_wire_invite_used");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let (mut bruno, bruno_id) = person(&url, "bruno@example.test");
    let (mut carla, _) = person(&url, "carla@example.test");
    let team = make_team(&mut ana, "Acme");

    let used = new_invite(&mut ana, &team);
    join_team(&mut bruno, &used).unwrap();
    admin_on(&hub.database, &format!("delete from accounts where id = '{bruno_id}'"));
    assert_eq!(db_text(&hub.database, "select count(*)::text from accounts where email = 'bruno@example.test'"), vec!["0"], "the joiner is gone");
    assert_eq!(db_text(&hub.database, "select (used_by is null)::text || ',' || (used_at is not null)::text from team_invites"), vec!["true,true"], "the invite lost its joiner but kept when it was spent");

    // (a) Another person with the same code is refused like an unknown code and is not a member.
    assert_eq!(join_team(&mut carla, &used), Err(NO_INVITE.into()));
    assert_eq!(team_op(&mut carla, &team, json!({ "op": "members" })), Err(NOT_A_MEMBER.into()));
    // A member of the team who tries the used code again is refused too, as it is not an open invite.
    assert_eq!(join_team(&mut ana, &used), Err(NO_INVITE.into()));
    assert_eq!(db_text(&hub.database, "select count(*)::text from team_members where team::text = (select id::text from accounts where kind = 'team')"), vec!["1"], "only the owner");

    // (b) The owner's list of open invites does not show it.
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "invites" })), Ok(json!([])));

    // The owner cannot revoke it either: it is not an unused invite.
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "revoke_invite", "hint": &used[..4] })), Err("there is no unused invite with that hint".into()));
    assert_eq!(db_text(&hub.database, "select revoked::text from team_invites"), vec!["false"], "and it was not marked revoked");

    // (c) It does not count toward the limit of 20 open invites: 19 others and one more are fine.
    for _ in 0..19 {
        new_invite(&mut ana, &team);
    }
    let last = team_op(&mut ana, &team, json!({ "op": "invite" })).unwrap();
    assert_eq!(last["hint"].as_str().map(str::len), Some(4), "the 20th open invite is made: {last}");
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "invite" })), Err(FULL.into()), "the 21st open one is refused");
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "invites" })).unwrap().as_array().unwrap().len(), 20);
}

#[test]
fn saving_a_new_bot_answers_its_workflow() {
    let hub = TestHub::start("teams_wire_saved");
    let (mut ana, _) = person(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");

    // An older app's save: no id, the hub names the workflow.
    let saved = team_op(&mut ana, &team, save_bot("")).unwrap();
    let id = saved["workflow"].as_str().unwrap_or_else(|| panic!("the workflow's id: {saved}")).to_owned();
    assert_eq!(saved, json!({ "workflow": id }));
    let listed = team_op(&mut ana, &team, json!({ "op": "bots" })).unwrap();
    assert_eq!(listed.as_array().unwrap().iter().map(|b| b["workflow"].clone()).collect::<Vec<_>>(), vec![json!(id)]);
    assert_eq!(team_workflows(&hub, Uuid::parse_str(&team).unwrap()).iter().map(|c| c["id"].clone()).collect::<Vec<_>>(), vec![json!(id)]);

    // An app that names its new workflow hears that name back.
    let fresh = Uuid::new_v4().to_string();
    assert_eq!(team_op(&mut ana, &team, save_bot(&fresh)), Ok(json!({ "workflow": fresh })));
    let ids: Vec<String> = team_workflows(&hub, Uuid::parse_str(&team).unwrap()).iter().map(|c| c["id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(ids, vec![id, fresh]);
}

#[test]
fn a_save_sent_again_after_its_bot_row_failed_keeps_one_workflow() {
    let hub = TestHub::start("teams_wire_retry");
    let (mut ana, _) = person(&hub.url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let team_id = Uuid::parse_str(&team).unwrap();
    let fresh = Uuid::new_v4().to_string();

    // The workflow is saved, then the bot's row waits on the held table and answers busy.
    let held = HeldBots::hold(&hub.database);
    let started = Instant::now();
    assert_eq!(team_op(&mut ana, &team, save_bot(&fresh)), Err(BUSY.into()));
    assert!(started.elapsed() < Duration::from_secs(10), "the lock wait is bounded: {:?}", started.elapsed());
    held.release();
    assert_eq!(team_workflows(&hub, team_id).len(), 1, "the workflow was kept before the row failed");

    // The app sends the same save again.
    assert_eq!(team_op(&mut ana, &team, save_bot(&fresh)), Ok(json!({ "workflow": fresh })));
    let ids: Vec<Value> = team_workflows(&hub, team_id).iter().map(|c| c["id"].clone()).collect();
    assert_eq!(ids, vec![json!(fresh)], "one workflow");
    let bots = team_op(&mut ana, &team, json!({ "op": "bots" })).unwrap();
    assert_eq!(bots.as_array().unwrap().iter().map(|b| b["workflow"].clone()).collect::<Vec<_>>(), vec![json!(fresh)], "one bot");
    assert_eq!(db_text(&hub.database, &format!("select workflow from team_bots where team = '{team}'")), vec![fresh.clone()], "one bot row");
}

#[test]
fn a_new_workflow_keeps_only_a_fresh_uuid_its_app_made() {
    let hub = TestHub::start("teams_wire_named");
    let (mut ana, _) = person(&hub.url, "ana@example.test");
    let first = make_team(&mut ana, "Acme");
    let second = make_team(&mut ana, "Beta");
    let fresh = Uuid::new_v4().to_string();
    assert_eq!(team_op(&mut ana, &first, save_bot(&fresh)), Ok(json!({ "workflow": fresh })));

    // Another team's workflow id, once its store is written, is no new id for this team.
    wait_for("the first team's store is written", || db_text(&hub.database, &format!("select count(*)::text from accounts where id = '{first}' and store::text like '%{fresh}%'")) == vec!["1"]);
    assert_eq!(team_op(&mut ana, &second, save_bot(&fresh)), Err(format!("this team has no workflow {fresh}")));
    // Not a uuid in its lowercase hyphenated form: unknown, as before.
    for odd in [fresh.to_uppercase(), fresh.replace('-', ""), "c1790000000000".to_owned()] {
        assert_eq!(team_op(&mut ana, &second, save_bot(&odd)), Err(format!("this team has no workflow {odd}")), "{odd}");
    }
    assert_eq!(team_workflows(&hub, Uuid::parse_str(&second).unwrap()), Vec::<Value>::new(), "nothing saved");
}
