//! What keeps a team's people and codes from being probed: an account id from outside a team is
//! refused like an unknown one, join codes are single-purpose, limited and capped, and
//! a member who goes takes its projects, group seats and bot ownership with it.

use super::teams_membership::{db_text, join_team, make_team, new_invite, person, roster, team_op};
use super::*;
use uuid::Uuid;

const NO_INVITE: &str = "that invite code is unknown, used or revoked";
const NOT_A_MEMBER: &str = "you are not a member of that team";
const TOO_MANY_JOINS: &str = "too many invite codes tried; try again in 15 minutes";

/// The refusal of an account id that is no member of this team, for the id as the client sent it.
fn no_member(id: &str) -> String {
    format!("no member {id} on this team")
}

#[test]
fn an_id_from_outside_the_team_is_refused_exactly_like_an_unknown_or_malformed_one() {
    let hub = TestHub::start("teams_probe");
    let url = hub.url.clone();
    let (mut ana, ana_id) = person(&url, "ana@example.test");
    let (mut bruno, bruno_id) = person(&url, "bruno@example.test");
    let (mut zed, zed_id) = person(&url, "zed@example.test");
    let team = make_team(&mut ana, "Acme");
    join_team(&mut bruno, &new_invite(&mut ana, &team)).unwrap();
    let other = make_team(&mut zed, "Zed's");
    assert_eq!(roster(&mut zed, &other).unwrap().as_array().unwrap().len(), 1, "zed is a real person on another team");

    let random = Uuid::new_v4().to_string();
    let probes = [zed_id.as_str(), random.as_str(), "not-an-account"];
    let group = |members: &[&str], fallback: Option<&str>| {
        json!({ "op": "save_group", "group": { "handle": "design", "about": "", "members": members, "fallback": fallback, "slack_group": "" } })
    };
    let mut refused: Vec<Vec<String>> = Vec::new();
    for (kind, ask) in [
        ("members", Box::new(|id: &str| group(&[ana_id.as_str(), id], None)) as Box<dyn Fn(&str) -> Value>),
        ("fallback", Box::new(|id: &str| group(&[], Some(id)))),
        ("remove_member", Box::new(|id: &str| json!({ "op": "remove_member", "account": id }))),
    ] {
        let errors: Vec<String> = probes.iter().map(|id| team_op(&mut ana, &team, ask(id)).unwrap_err().replace(id, "<id>")).collect();
        for (id, error) in probes.iter().zip(&errors) {
            assert_eq!(error, "no member <id> on this team", "{kind} {id}");
        }
        for id in probes {
            assert_eq!(team_op(&mut ana, &team, ask(id)), Err(no_member(id)), "{kind} {id}");
        }
        refused.push(errors);
    }
    assert!(refused.iter().flatten().all(|e| e == &refused[0][0]), "one message for every kind of wrong id");
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "groups" })), Ok(json!([])), "no refused save stored anything");
    assert_eq!(roster(&mut zed, &other).unwrap().as_array().unwrap().len(), 1);

    // A member's save ignores the members it sends, but the ids are still checked: no role probes.
    for id in probes {
        assert_eq!(team_op(&mut bruno, &team, group(&[id], None)), Err(no_member(id)), "a member's save of {id}");
    }
    // Real members of this team pass, in members and as fallback.
    let ok = team_op(&mut ana, &team, group(&[ana_id.as_str(), bruno_id.as_str(), bruno_id.as_str()], Some(&bruno_id)));
    assert_eq!(ok, Ok(Value::Null));
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "remove_member", "account": bruno_id })), Ok(Value::Null));
}

#[test]
fn a_non_member_gets_no_member_list_and_a_persons_team_list_carries_no_emails() {
    let hub = TestHub::start("teams_emails");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let (mut bruno, _) = person(&url, "bruno@example.test");
    let (mut carla, _) = person(&url, "carla@example.test");
    let team = make_team(&mut ana, "Acme");
    join_team(&mut bruno, &new_invite(&mut ana, &team)).unwrap();

    assert_eq!(roster(&mut carla, &team), Err(NOT_A_MEMBER.into()), "the scope error, no list");
    assert_eq!(roster(&mut bruno, &team).unwrap().as_array().unwrap().len(), 2, "a member reads it");
    for client in [&mut ana, &mut bruno, &mut carla] {
        let teams = client.hub(json!({ "cmd": "team", "op": { "op": "list" } })).unwrap();
        let text = teams.to_string();
        assert!(!text.contains("@example.test") && !text.contains("email") && !text.contains("members"), "{text}");
    }
}

#[test]
fn an_account_signup_code_is_never_a_team_code_and_a_team_code_never_signs_anyone_up() {
    let hub = TestHub::start_with("teams_purpose", Signup::Invite);
    let url = hub.url.clone();
    let (status, ana) = signup_from(&url, "10.0.0.1", json!({ "email": "ana@example.test", "password": "correct horse" }));
    assert_eq!(status, 200, "{ana}");
    let mut ana_client = signed_in_client(&url, ana["token"].as_str().unwrap());
    let signup_code = ana_client.hub(json!({ "cmd": "create_invite" })).unwrap()["code"].as_str().unwrap().to_owned();
    let spare_signup_code = ana_client.hub(json!({ "cmd": "create_invite" })).unwrap()["code"].as_str().unwrap().to_owned();
    let team = make_team(&mut ana_client, "Acme");
    let team_code = new_invite(&mut ana_client, &team);

    let (status, bruno) = signup_from(&url, "10.0.0.2", json!({ "email": "bruno@example.test", "password": "correct horse", "invite": spare_signup_code }));
    assert_eq!(status, 200, "{bruno}");
    let mut bruno_client = signed_in_client(&url, bruno["token"].as_str().unwrap());

    assert_eq!(join_team(&mut bruno_client, &signup_code), Err(NO_INVITE.into()), "a signup code opens no team");
    assert_eq!(signup_from(&url, "10.0.0.3", json!({ "email": "carla@example.test", "password": "correct horse", "invite": team_code })), refused_signup(), "a team code makes no account");
    assert_eq!(db_text(&hub.database, "select count(*)::text from accounts where kind = 'person'"), vec!["2"]);

    // Neither refusal spent a code: each still does its own job.
    assert_eq!(signup_from(&url, "10.0.0.3", json!({ "email": "carla@example.test", "password": "correct horse", "invite": signup_code })).0, 200);
    assert_eq!(join_team(&mut bruno_client, &team_code).unwrap()["id"], json!(team));
}

#[test]
fn failed_joins_stop_at_10_per_account_without_reaching_the_database_and_a_landed_join_does_not_count() {
    let hub = TestHub::start("teams_join_limit");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let (mut bruno, _) = person(&url, "bruno@example.test");
    let (mut carla, carla_id) = person(&url, "carla@example.test");
    let (mut dani, _) = person(&url, "dani@example.test");
    let team = make_team(&mut ana, "Acme");
    let for_bruno = new_invite(&mut ana, &team);
    let for_carla = new_invite(&mut ana, &team);
    let for_dani = new_invite(&mut ana, &team);
    let guess = |n: usize| format!("ZZZZ-ZZZZ-ZZZZ-{n:04}");

    for n in 0..9 {
        assert_eq!(join_team(&mut bruno, &guess(n)), Err(NO_INVITE.into()), "guess {n}");
    }
    assert_eq!(join_team(&mut bruno, &for_bruno).unwrap()["id"], json!(team));
    assert_eq!(join_team(&mut bruno, &guess(9)), Err(NO_INVITE.into()), "9 failures and a join that landed: the 10th still answers the code");
    assert_eq!(join_team(&mut bruno, &guess(10)), Err(TOO_MANY_JOINS.into()), "the landed join did not count; the 11th try is refused");

    for n in 0..10 {
        assert_eq!(join_team(&mut carla, &guess(n)), Err(NO_INVITE.into()), "guess {n}");
    }
    assert_eq!(join_team(&mut carla, &for_carla), Err(TOO_MANY_JOINS.into()), "past the limit even a good code is refused");
    let state = |code: &str| db_text(&hub.database, &format!("select coalesce(used_by::text, 'unused') from team_invites where code_hash = sha256(convert_to('{}', 'UTF8'))", code.replace('-', "")));
    assert_eq!(state(&for_carla), vec!["unused"], "the refused try never reached the invite");
    assert_eq!(db_text(&hub.database, &format!("select count(*)::text from team_members where member::text = '{carla_id}'")), vec!["0"]);
    assert_eq!(roster(&mut carla, &team), Err(NOT_A_MEMBER.into()));

    assert_eq!(join_team(&mut dani, &for_dani).unwrap()["id"], json!(team), "another account is not held by carla's failures");
}

/// A team with its owner Ana and members Bruno and Carla, who each attached a project, sit in the
/// `design` group, own a bot, and (Carla) is its fallback; Ana's bot is the control.
fn crowded(name: &str) -> (TestHub, String, Client, (Client, String), (Client, String), String) {
    let hub = TestHub::start(name);
    let url = hub.url.clone();
    let (mut ana, ana_id) = person(&url, "ana@example.test");
    let (mut bruno, bruno_id) = person(&url, "bruno@example.test");
    let (mut carla, carla_id) = person(&url, "carla@example.test");
    let team = make_team(&mut ana, "Acme");
    join_team(&mut bruno, &new_invite(&mut ana, &team)).unwrap();
    join_team(&mut carla, &new_invite(&mut ana, &team)).unwrap();
    admin_on(
        &hub.database,
        &format!(
            "insert into team_projects (team, key, name, repo, member) values
                ('{team}', 'github.com/acme/site', 'site', 'github.com/acme/site', '{bruno_id}'),
                ('{team}', 'github.com/acme/api', 'api', 'github.com/acme/api', '{carla_id}');
             insert into team_groups (team, handle, fallback) values ('{team}', 'design', '{carla_id}');
             insert into team_group_members (team, handle, member) values ('{team}', 'design', '{bruno_id}'), ('{team}', 'design', '{carla_id}');
             insert into team_bots (team, workflow, owner, calls, limits) values
                ('{team}', 'bruno-bot', '{bruno_id}', '[]'::jsonb, '{{}}'::jsonb),
                ('{team}', 'carla-bot', '{carla_id}', '[]'::jsonb, '{{}}'::jsonb),
                ('{team}', 'ana-bot', '{ana_id}', '[]'::jsonb, '{{}}'::jsonb);"
        ),
    );
    (hub, team, ana, (bruno, bruno_id), (carla, carla_id), ana_id)
}

fn bot_owners(hub: &TestHub, team: &str) -> Vec<String> {
    db_text(&hub.database, &format!("select workflow || '=' || coalesce(owner::text, 'none') from team_bots where team::text = '{team}' order by workflow"))
}

#[test]
fn a_member_who_leaves_takes_its_projects_and_group_seats_and_its_bots_lose_their_owner() {
    let (hub, team, _ana, (mut bruno, _), (_carla, carla_id), ana_id) = crowded("teams_leave_sweep");

    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "leave" })), Ok(Value::Null));
    assert_eq!(db_text(&hub.database, &format!("select key from team_projects where team::text = '{team}'")), vec!["github.com/acme/api"], "only its own project went");
    assert_eq!(db_text(&hub.database, &format!("select member::text from team_group_members where team::text = '{team}'")), vec![carla_id.clone()]);
    assert_eq!(bot_owners(&hub, &team), vec!["ana-bot=".to_owned() + &ana_id, "bruno-bot=none".into(), "carla-bot=".to_owned() + &carla_id]);
    assert_eq!(db_text(&hub.database, &format!("select fallback::text from team_groups where team::text = '{team}'")), vec![carla_id], "bruno was not the fallback");
}

#[test]
fn a_member_who_is_the_fallback_leaves_and_the_group_forgets_it_and_a_removed_members_bots_lose_their_owner() {
    let (hub, team, mut ana, (_bruno, bruno_id), (mut carla, _), ana_id) = crowded("teams_remove_sweep");

    assert_eq!(team_op(&mut carla, &team, json!({ "op": "leave" })), Ok(Value::Null));
    assert_eq!(db_text(&hub.database, &format!("select key from team_projects where team::text = '{team}'")), vec!["github.com/acme/site"]);
    assert_eq!(db_text(&hub.database, &format!("select (fallback is null)::text from team_groups where team::text = '{team}'")), vec!["true"], "leaving clears the fallback");
    assert_eq!(bot_owners(&hub, &team), vec!["ana-bot=".to_owned() + &ana_id, "bruno-bot=".to_owned() + &bruno_id, "carla-bot=none".into()]);

    assert_eq!(team_op(&mut ana, &team, json!({ "op": "remove_member", "account": bruno_id })), Ok(Value::Null));
    assert_eq!(bot_owners(&hub, &team), vec!["ana-bot=".to_owned() + &ana_id, "bruno-bot=none".into(), "carla-bot=none".into()], "removing clears it too");
    assert_eq!(db_text(&hub.database, &format!("select count(*)::text from team_projects where team::text = '{team}'")), vec!["0"]);
}
