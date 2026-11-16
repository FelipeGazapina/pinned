//! Team membership on the hub: invites, joining, the member list, owner-only calls, leaving and removing,
//! and the refusals that keep a team's account from ever signing in, pairing or changing a password.

use super::*;
use uuid::Uuid;

const NO_INVITE: &str = "that invite code is unknown, used or revoked";
const NOT_A_MEMBER: &str = "you are not a member of that team";
const NOT_OWNER: &str = "only the team's owner does this";
const NOT_ON_TEAM: &str = "that person is not a member of the team";
const OWNER_LEAVES: &str = "the owner cannot leave the team; remove the others first";
const OWNER_REMOVED: &str = "the owner cannot be removed from the team";
const NO_UNUSED_INVITE: &str = "there is no unused invite with that hint";
const SIGN_IN_FIRST: &str = "sign in first";
const WRONG_LOGIN: &str = "wrong email or password";
const UNKNOWN_PAIR_CODE: &str = "that pairing code is unknown, used or expired";

/// A person who signed up as `email` and is signed in on the hub, with its account id.
fn person(url: &str, email: &str) -> (Client, String) {
    let (token, view) = account::signup(url, email, "correct horse").unwrap();
    (signed_in_client(url, &token), view.id)
}

/// One member's call of `op` on `team`, answered as the hub answers it.
fn team_op(client: &mut Client, team: &str, op: Value) -> Result<Value, String> {
    client.hub(json!({ "cmd": "team", "team": team, "op": op }))
}

/// The owner makes a team named `name`; its id.
fn make_team(owner: &mut Client, name: &str) -> String {
    let team = owner.hub(json!({ "cmd": "team", "op": { "op": "create", "name": name } })).unwrap();
    team["id"].as_str().unwrap().to_owned()
}

/// A new invite code the owner makes, shown once.
fn new_invite(owner: &mut Client, team: &str) -> String {
    let invite = team_op(owner, team, json!({ "op": "invite" })).unwrap();
    invite["code"].as_str().unwrap().to_owned()
}

/// Joins the team of `code` as `client`: the team as the hub shows it to the joiner.
fn join_team(client: &mut Client, code: &str) -> Result<Value, String> {
    client.hub(json!({ "cmd": "team", "op": { "op": "join", "code": code } }))
}

/// The team's members as one of them reads them.
fn roster(client: &mut Client, team: &str) -> Result<Value, String> {
    team_op(client, team, json!({ "op": "members" }))
}

/// The first column of each row `sql` returns on the hub's database, as text.
fn db_text(database: &str, sql: &str) -> Vec<String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&format!("{}/{database}", postgres()), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        let rows = client.query(sql, &[]).await.unwrap();
        rows.iter().map(|row| row.get::<_, String>(0)).collect()
    })
}

/// How many logins the team's account has, which must be none.
fn team_logins(database: &str, team: &str) -> Vec<String> {
    db_text(database, &format!("select count(*)::text from logins where account::text = '{team}'"))
}

#[test]
fn an_owner_invites_a_member_who_joins_and_both_see_two_members() {
    let hub = TestHub::start("teams_invite");
    let url = hub.url.clone();
    let (mut ana, ana_id) = person(&url, "ana@example.test");
    let (mut bruno, bruno_id) = person(&url, "bruno@example.test");
    let team = make_team(&mut ana, "Acme");

    let invite = team_op(&mut ana, &team, json!({ "op": "invite" })).unwrap();
    let code = invite["code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 19, "four groups of four: {code}");
    assert_eq!(invite["hint"], json!(&code[..4]));

    let joined = join_team(&mut bruno, &code).unwrap();
    assert_eq!(
        joined,
        json!({ "id": team, "name": "Acme", "owner": ana_id, "role": "member",
            "credential": { "upstash": false, "claude": false, "github": false, "slack": false } })
    );

    let members = json!([
        { "account": ana_id, "email": "ana@example.test", "role": "owner", "groups": [] },
        { "account": bruno_id, "email": "bruno@example.test", "role": "member", "groups": [] },
    ]);
    assert_eq!(roster(&mut ana, &team), Ok(members.clone()));
    assert_eq!(roster(&mut bruno, &team), Ok(members));
    // The hub keeps the code's hash of its letters and digits, never the code: the one row is that hash.
    let letters: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    assert_eq!(db_text(&hub.database, &format!("select (code_hash = sha256(convert_to('{letters}', 'UTF8')))::text from team_invites")), vec!["true"]);
}

#[test]
fn a_used_a_revoked_and_an_unknown_code_each_give_the_same_error_and_the_stranger_stays_out() {
    let hub = TestHub::start("teams_codes");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let (mut bruno, _) = person(&url, "bruno@example.test");
    let (mut carla, _) = person(&url, "carla@example.test");
    let team = make_team(&mut ana, "Acme");
    let used = new_invite(&mut ana, &team);
    let revoked = new_invite(&mut ana, &team);
    join_team(&mut bruno, &used).unwrap();
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "revoke_invite", "hint": &revoked[..4] })), Ok(Value::Null));

    let unknown = "ZZZZ-ZZZZ-ZZZZ-ZZZZ";
    for code in [&used, &revoked, unknown] {
        assert_eq!(join_team(&mut carla, code), Err(NO_INVITE.into()), "code {code}");
    }
    assert_eq!(roster(&mut carla, &team), Err(NOT_A_MEMBER.into()));
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "revoke_invite", "hint": &revoked[..4] })), Err(NO_UNUSED_INVITE.into()));
}

#[test]
fn two_accounts_racing_one_invite_code_see_exactly_one_join() {
    let hub = TestHub::start("teams_race");
    let url = hub.url.clone();
    let (mut ana, ana_id) = person(&url, "ana@example.test");
    let racers: Vec<(&str, Client, String)> = ["bruno@example.test", "carla@example.test"]
        .into_iter()
        .map(|email| {
            let (client, id) = person(&url, email);
            (email, client, id)
        })
        .collect();
    let team = make_team(&mut ana, "Acme");
    let code = new_invite(&mut ana, &team);

    let start = std::sync::Barrier::new(racers.len());
    let results: Vec<(&str, String, Client, Result<Value, String>)> = std::thread::scope(|s| {
        let handles: Vec<_> = racers
            .into_iter()
            .map(|(email, mut client, id)| {
                let (start, code) = (&start, &code);
                s.spawn(move || {
                    start.wait();
                    let result = join_team(&mut client, code);
                    (email, id, client, result)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let (mut won, mut lost): (Vec<_>, Vec<_>) = results.into_iter().partition(|(_, _, _, result)| result.is_ok());
    assert_eq!(won.len(), 1, "exactly one racer joins");
    assert_eq!(lost.len(), 1, "exactly one racer is refused");
    let (winner_email, winner_id, _, winner_joined) = won.remove(0);
    assert_eq!(winner_joined.unwrap()["role"], json!("member"));
    let (_, _, mut loser, loser_joined) = lost.remove(0);
    assert_eq!(loser_joined, Err(NO_INVITE.into()));
    assert_eq!(roster(&mut loser, &team), Err(NOT_A_MEMBER.into()));

    assert_eq!(
        roster(&mut ana, &team),
        Ok(json!([
            { "account": ana_id, "email": "ana@example.test", "role": "owner", "groups": [] },
            { "account": winner_id, "email": winner_email, "role": "member", "groups": [] },
        ]))
    );
}

#[test]
fn a_member_who_is_not_the_owner_is_refused_each_owner_only_call() {
    let hub = TestHub::start("teams_owner_only");
    let url = hub.url.clone();
    let (mut ana, ana_id) = person(&url, "ana@example.test");
    let (mut bruno, _) = person(&url, "bruno@example.test");
    let team = make_team(&mut ana, "Acme");
    join_team(&mut bruno, &new_invite(&mut ana, &team)).unwrap();

    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "invite" })), Err(NOT_OWNER.into()));
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "revoke_invite", "hint": "ABCD" })), Err(NOT_OWNER.into()));
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "rename", "name": "Hacked" })), Err(NOT_OWNER.into()));
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "remove_member", "account": ana_id })), Err(NOT_OWNER.into()));

    let listed = ana.hub(json!({ "cmd": "team", "op": { "op": "list" } })).unwrap();
    assert_eq!(listed[0]["name"], json!("Acme"), "the refused rename changed nothing");
    // The owner does the same call and it lands, so the refusals above are the role's doing.
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "rename", "name": "Acme 2" })), Ok(Value::Null));
    let listed = ana.hub(json!({ "cmd": "team", "op": { "op": "list" } })).unwrap();
    assert_eq!(listed[0]["name"], json!("Acme 2"));
}

#[test]
fn a_member_who_leaves_loses_the_team_and_the_owner_removes_another_and_cannot_leave() {
    let hub = TestHub::start("teams_leave");
    let url = hub.url.clone();
    let (mut ana, ana_id) = person(&url, "ana@example.test");
    let (mut bruno, _) = person(&url, "bruno@example.test");
    let (mut carla, carla_id) = person(&url, "carla@example.test");
    let team = make_team(&mut ana, "Acme");
    join_team(&mut bruno, &new_invite(&mut ana, &team)).unwrap();
    join_team(&mut carla, &new_invite(&mut ana, &team)).unwrap();

    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "leave" })), Ok(Value::Null));
    assert_eq!(roster(&mut bruno, &team), Err(NOT_A_MEMBER.into()), "the very next call is refused");
    assert_eq!(bruno.hub(json!({ "cmd": "team", "op": { "op": "list" } })), Ok(json!([])));
    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "leave" })), Err(NOT_A_MEMBER.into()));

    assert_eq!(team_op(&mut ana, &team, json!({ "op": "remove_member", "account": carla_id })), Ok(Value::Null));
    assert_eq!(roster(&mut carla, &team), Err(NOT_A_MEMBER.into()));
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "remove_member", "account": carla_id })), Err(NOT_ON_TEAM.into()));
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "remove_member", "account": "not-an-account" })), Err(NOT_ON_TEAM.into()));

    assert_eq!(team_op(&mut ana, &team, json!({ "op": "leave" })), Err(OWNER_LEAVES.into()));
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "remove_member", "account": ana_id })), Err(OWNER_REMOVED.into()));
    assert_eq!(
        roster(&mut ana, &team),
        Ok(json!([{ "account": ana_id, "email": "ana@example.test", "role": "owner", "groups": [] }])),
        "bruno and carla are gone, the owner stays"
    );
}

#[test]
fn a_person_who_is_not_a_member_and_a_malformed_team_id_are_both_refused() {
    let hub = TestHub::start("teams_outsider");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let (mut carla, _) = person(&url, "carla@example.test");
    let team = make_team(&mut ana, "Acme");

    assert_eq!(roster(&mut carla, &team), Err(NOT_A_MEMBER.into()));
    assert_eq!(roster(&mut carla, "00000000-0000-0000-0000-000000000000"), Err(NOT_A_MEMBER.into()));
    assert_eq!(roster(&mut ana, "not-a-team"), Err("not-a-team is not a team id".into()));
    assert_eq!(roster(&mut carla, "not-a-team"), Err("not-a-team is not a team id".into()));
}

#[test]
fn removing_a_member_takes_its_projects_and_group_seats_and_clears_it_as_a_fallback() {
    let hub = TestHub::start("teams_cascade");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
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
             insert into team_group_members (team, handle, member) values ('{team}', 'design', '{bruno_id}'), ('{team}', 'design', '{carla_id}');"
        ),
    );

    assert_eq!(team_op(&mut bruno, &team, json!({ "op": "leave" })), Ok(Value::Null));
    assert_eq!(team_op(&mut ana, &team, json!({ "op": "remove_member", "account": carla_id })), Ok(Value::Null));

    assert_eq!(db_text(&hub.database, &format!("select key from team_projects where team::text = '{team}'")), Vec::<String>::new());
    assert_eq!(db_text(&hub.database, &format!("select member::text from team_group_members where team::text = '{team}'")), Vec::<String>::new());
    assert_eq!(db_text(&hub.database, &format!("select (fallback is null)::text from team_groups where team::text = '{team}'")), vec!["true"]);
    assert_eq!(db_text(&hub.database, &format!("select handle from team_groups where team::text = '{team}'")), vec!["design"], "the group itself stays");
}

#[test]
fn a_team_account_cannot_sign_up_with_its_own_email() {
    let hub = TestHub::start("teams_signup");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let team_email = format!("team-{team}@teams.invalid");

    assert_eq!(
        post_with(&url, "/api/signup", &[], json!({ "email": team_email, "password": "correct horse" })),
        (409, json!({ "error": "an account with that email already exists" }))
    );
    assert_eq!(team_logins(&hub.database, &team), vec!["0"]);
}

#[test]
fn a_team_account_cannot_sign_in_with_its_email_and_any_password() {
    let hub = TestHub::start("teams_login");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let team_email = format!("team-{team}@teams.invalid");
    assert_eq!(db_text(&hub.database, &format!("select email from accounts where id::text = '{team}'")), vec![team_email.clone()]);

    for password in ["any password", "!"] {
        let as_team = post_with(&url, "/api/login", &[], json!({ "email": team_email, "password": password }));
        let as_stranger = post_with(&url, "/api/login", &[], json!({ "email": "nobody@example.test", "password": password }));
        assert_eq!(as_team, (401, json!({ "error": WRONG_LOGIN })));
        assert_eq!(as_team, as_stranger, "a team's email answers exactly as an unknown email does");
    }
    assert_eq!(account::login(&url, &team_email, "any password").map(|_| ()), Err(WRONG_LOGIN.into()));
    assert_eq!(team_logins(&hub.database, &team), vec!["0"]);
}

#[test]
fn the_db_refuses_to_make_a_login_for_a_team_account() {
    let hub = TestHub::start("teams_db_login");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");

    let made = hub.runtime.block_on(hub.hub().db.create_login(Uuid::parse_str(&team).unwrap())).map(|_| ());
    assert_eq!(made, Err(format!("account {team} cannot sign in")));
    assert_eq!(team_logins(&hub.database, &team), vec!["0"]);
}

#[test]
fn a_team_pair_code_cannot_pair_a_machine() {
    let hub = TestHub::start("teams_pair");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    admin_on(&hub.database, &format!("insert into pair_codes (code, account, expires_at) values ('TEAMPAIR', '{team}', now() + interval '15 minutes')"));

    assert_eq!(post_with(&url, "/api/pair", &[], json!({ "code": "TEAMPAIR", "name": "vps", "os": "linux" })), (404, json!({ "error": UNKNOWN_PAIR_CODE })));
    assert_eq!(db_text(&hub.database, "select count(*)::text from machines"), vec!["0"]);
    assert_eq!(team_logins(&hub.database, &team), vec!["0"]);
}

#[test]
fn a_team_cannot_change_its_password_because_no_login_can_be_made_for_it() {
    let hub = TestHub::start("teams_password");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");
    let team_email = format!("team-{team}@teams.invalid");
    // The routes that could give the team a login, tried first: a sign in, a made login, a pairing.
    assert_eq!(post_with(&url, "/api/login", &[], json!({ "email": team_email, "password": "any password" })).0, 401);
    assert_eq!(hub.runtime.block_on(hub.hub().db.create_login(Uuid::parse_str(&team).unwrap())).map(|_| ()), Err(format!("account {team} cannot sign in")));
    admin_on(&hub.database, &format!("insert into pair_codes (code, account, expires_at) values ('TEAMPAIR', '{team}', now() + interval '15 minutes')"));
    assert_eq!(post_with(&url, "/api/pair", &[], json!({ "code": "TEAMPAIR", "name": "vps", "os": "linux" })).0, 404);

    let change = json!({ "current": "any password", "password": "battery staple" });
    assert_eq!(post_with(&url, "/api/password", &[], change.clone()), (401, json!({ "error": SIGN_IN_FIRST })), "no token");
    assert_eq!(
        post_with(&url, "/api/password", &[("authorization", "Bearer not-a-login-token")], change),
        (401, json!({ "error": SIGN_IN_FIRST })),
        "a forged token"
    );
    assert_eq!(db_text(&hub.database, &format!("select password_hash from accounts where id::text = '{team}'")), vec!["!"]);
    assert_eq!(team_logins(&hub.database, &team), vec!["0"]);
}

#[test]
fn a_team_cannot_register_a_machine_on_the_machines_route() {
    let hub = TestHub::start("teams_machines");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");

    assert_eq!(post_with(&url, "/api/machines", &[], json!({ "name": "vps", "os": "linux" })), (401, json!({ "error": SIGN_IN_FIRST })));
    assert_eq!(db_text(&hub.database, "select count(*)::text from machines"), vec!["0"]);
    assert_eq!(team_logins(&hub.database, &team), vec!["0"]);
}

#[test]
fn a_team_cannot_get_an_agent_token_from_the_token_route_or_an_approval() {
    let hub = TestHub::start("teams_oauth");
    let url = hub.url.clone();
    let (mut ana, _) = person(&url, "ana@example.test");
    let team = make_team(&mut ana, "Acme");

    // A grant comes from a person's approval in the browser; the team's own call of it is refused.
    let approve = json!({ "op": "hub", "call": { "cmd": "approve_agent", "request": "r1", "scopes": [], "projects": [] } });
    assert_eq!(team_op(&mut ana, &team, approve), Err("approve_agent does not run on a team".into()));
    // The token route has no grant for an unknown code.
    let exchange = json!({ "grant_type": "authorization_code", "client_id": "agent-x", "code": "ZZZZ", "code_verifier": "v", "redirect_uri": "http://localhost/cb" });
    assert_eq!(
        post_with(&url, "/oauth/token", &[], exchange),
        (400, json!({ "error": "invalid_grant", "error_description": "unknown code" }))
    );
    assert_eq!(db_text(&hub.database, &format!("select count(*)::text from agent_grants where account::text = '{team}'")), vec!["0"]);
}

#[test]
fn a_member_who_joins_again_keeps_its_role_and_the_code_stays_unused() {
    let hub = TestHub::start("teams_rejoin");
    let url = hub.url.clone();
    let (mut ana, ana_id) = person(&url, "ana@example.test");
    let (mut bruno, bruno_id) = person(&url, "bruno@example.test");
    let team = make_team(&mut ana, "Acme");
    let first = new_invite(&mut ana, &team);
    let spare = new_invite(&mut ana, &team);
    join_team(&mut bruno, &first).unwrap();

    // The owner spends its own code on the team it owns: it stays the owner, and the code is still unused.
    assert_eq!(join_team(&mut ana, &spare).unwrap()["role"], json!("owner"));
    // A member spending a second code keeps its member role, and that code stays unused too.
    assert_eq!(join_team(&mut bruno, &spare).unwrap()["role"], json!("member"));
    assert_eq!(db_text(&hub.database, &format!("select count(*)::text from team_invites where used_by is null and hint = '{}'", &spare[..4])), vec!["1"], "the spare code is still unused");
    assert_eq!(db_text(&hub.database, "select count(*)::text from team_invites where used_by is not null"), vec!["1"], "only the first code was spent");
    assert_eq!(
        roster(&mut ana, &team),
        Ok(json!([
            { "account": ana_id, "email": "ana@example.test", "role": "owner", "groups": [] },
            { "account": bruno_id, "email": "bruno@example.test", "role": "member", "groups": [] },
        ]))
    );
}

#[test]
fn members_show_each_members_group_handles_in_order() {
    let hub = TestHub::start("teams_groups");
    let url = hub.url.clone();
    let (mut ana, ana_id) = person(&url, "ana@example.test");
    let (mut bruno, bruno_id) = person(&url, "bruno@example.test");
    let team = make_team(&mut ana, "Acme");
    join_team(&mut bruno, &new_invite(&mut ana, &team)).unwrap();
    admin_on(
        &hub.database,
        &format!(
            "insert into team_groups (team, handle) values ('{team}', 'design'), ('{team}', 'backend');
             insert into team_group_members (team, handle, member) values
                ('{team}', 'design', '{bruno_id}'), ('{team}', 'backend', '{bruno_id}'), ('{team}', 'design', '{ana_id}');"
        ),
    );

    assert_eq!(
        roster(&mut bruno, &team),
        Ok(json!([
            { "account": ana_id, "email": "ana@example.test", "role": "owner", "groups": ["design"] },
            { "account": bruno_id, "email": "bruno@example.test", "role": "member", "groups": ["backend", "design"] },
        ]))
    );
}
