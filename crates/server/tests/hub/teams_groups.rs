//! A team's groups through the hub: who makes them, who sets their members, and who joins or
//! leaves. Every name and address here is invented (`.test`).

use super::*;

/// A hub with an Acme team: its owner, a member of it, and a stranger on no team, all signed in.
struct Acme {
    hub: TestHub,
    team: String,
    owner: Client,
    owner_id: String,
    member: Client,
    member_id: String,
    stranger: Client,
    stranger_id: String,
}

/// The owner makes the team. The member is put on it by an insert, since the membership unit
/// (invites, joining) is not part of this one.
fn acme(name: &str) -> Acme {
    let hub = TestHub::start(name);
    let url = hub.url.clone();
    let (owner_token, owner) = account::signup(&url, "owner@acme.test", "correct horse").unwrap();
    let (member_token, member) = account::signup(&url, "member@acme.test", "correct horse").unwrap();
    let (stranger_token, stranger) = account::signup(&url, "stranger@acme.test", "correct horse").unwrap();
    let mut owner_client = signed_in_client(&url, &owner_token);
    let made = owner_client.hub(json!({ "cmd": "team", "op": { "op": "create", "name": "Acme" } })).unwrap();
    let team = made["id"].as_str().unwrap().to_owned();
    admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{}', 'member')", member.id));
    Acme {
        team,
        owner: owner_client,
        owner_id: owner.id,
        member: signed_in_client(&url, &member_token),
        member_id: member.id,
        stranger: signed_in_client(&url, &stranger_token),
        stranger_id: stranger.id,
        hub,
    }
}

/// A team op as `client` sends it, on `team`.
fn op(client: &mut Client, team: &str, op: Value) -> Result<Value, String> {
    client.hub(json!({ "cmd": "team", "team": team, "op": op }))
}

/// The `save_group` op for a group `handle` with `about` and `members`, no fallback or Slack group.
fn save(handle: &str, about: &str, members: &[&str]) -> Value {
    json!({ "op": "save_group", "group": { "handle": handle, "about": about, "members": members, "fallback": null, "slack_group": "" } })
}

/// A group as `groups` shows it.
fn shown(handle: &str, about: &str, members: &[&str], fallback: Option<&str>) -> Value {
    json!({ "handle": handle, "about": about, "members": members, "fallback": fallback, "slack_group": "" })
}

/// Member ids sorted, as `groups` lists them.
fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

const NOT_A_MEMBER: &str = "you are not a member of that team";

#[test]
fn a_member_makes_a_group_under_its_handle_and_is_its_only_member() {
    let mut a = acme("groups_make");
    let asked = save("@design", "UI calls", &[a.owner_id.as_str(), a.member_id.as_str()]);
    assert_eq!(op(&mut a.member, &a.team, asked), Ok(Value::Null));
    let member = a.member_id.clone();
    assert_eq!(op(&mut a.member, &a.team, json!({ "op": "groups" })), Ok(json!([shown("design", "UI calls", &[member.as_str()], None)])));
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(json!([shown("design", "UI calls", &[member.as_str()], None)])), "the owner reads the same group");
}

#[test]
fn a_bad_handle_is_refused_with_one_clear_error() {
    let mut a = acme("groups_handles");
    let refused = "a group's handle is 2 to 32 lowercase letters, digits or dashes, like @design";
    let long = "a".repeat(33);
    for handle in ["A", "with space", long.as_str()] {
        assert_eq!(op(&mut a.owner, &a.team, save(handle, "", &[])), Err(refused.into()), "{handle}");
    }
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(json!([])), "a refused handle stores nothing");
    let edge = "a".repeat(32);
    assert_eq!(op(&mut a.owner, &a.team, save("ab", "", &[])), Ok(Value::Null), "2 characters is the shortest");
    assert_eq!(op(&mut a.owner, &a.team, save(&edge, "", &[])), Ok(Value::Null), "32 characters is the longest");
}

#[test]
fn the_owner_sets_the_members_and_a_members_edit_keeps_them() {
    let mut a = acme("groups_owner");
    let both = sorted(vec![a.owner_id.clone(), a.member_id.clone()]);
    let listed = |about: &str| json!([{ "handle": "design", "about": about, "members": both, "fallback": null, "slack_group": "" }]);
    assert_eq!(op(&mut a.owner, &a.team, save("@design", "", &[a.owner_id.as_str(), a.member_id.as_str()])), Ok(Value::Null));
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(listed("")));

    assert_eq!(op(&mut a.member, &a.team, save("@design", "UI calls", &[])), Ok(Value::Null), "a member edits the description and sends no members");
    assert_eq!(op(&mut a.member, &a.team, json!({ "op": "groups" })), Ok(listed("UI calls")), "the members stay [owner, member]");
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(listed("UI calls")));

    let stranger = a.stranger_id.clone();
    let owner = a.owner_id.clone();
    let refused = op(&mut a.owner, &a.team, save("@design", "", &[owner.as_str(), stranger.as_str()]));
    assert_eq!(refused, Err("@design has someone who is not a member of this team".into()), "the foreign key on team_group_members refuses a stranger");
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(listed("UI calls")), "the refused save changes nothing");
}

#[test]
fn a_member_cannot_put_other_people_into_a_group() {
    let mut a = acme("groups_members");
    let owner = a.owner_id.clone();
    let member = a.member_id.clone();
    let stranger = a.stranger_id.clone();
    assert_eq!(op(&mut a.owner, &a.team, save("@design", "", &[owner.as_str()])), Ok(Value::Null));

    assert_eq!(op(&mut a.member, &a.team, save("@design", "", &[member.as_str(), stranger.as_str()])), Ok(Value::Null), "the save is kept, and its members are not");
    assert_eq!(op(&mut a.member, &a.team, json!({ "op": "groups" })), Ok(json!([shown("design", "", &[owner.as_str()], None)])), "the stored list stays [owner]");

    assert_eq!(op(&mut a.member, &a.team, save("@ops", "", &[owner.as_str(), stranger.as_str()])), Ok(Value::Null));
    assert_eq!(
        op(&mut a.member, &a.team, json!({ "op": "groups" })),
        Ok(json!([shown("design", "", &[owner.as_str()], None), shown("ops", "", &[member.as_str()], None)])),
        "a group a member makes starts with that member only"
    );
}

#[test]
fn a_member_joins_and_leaves_a_group_by_itself() {
    let mut a = acme("groups_join");
    let owner = a.owner_id.clone();
    let member = a.member_id.clone();
    assert_eq!(op(&mut a.owner, &a.team, save("@design", "", &[owner.as_str()])), Ok(Value::Null));

    assert_eq!(op(&mut a.member, &a.team, json!({ "op": "join_group", "handle": "design" })), Ok(Value::Null));
    let both = sorted(vec![owner.clone(), member.clone()]);
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(json!([shown("design", "", &refs(&both), None)])));
    assert_eq!(op(&mut a.member, &a.team, json!({ "op": "join_group", "handle": "design" })), Ok(Value::Null), "joining twice changes nothing");

    assert_eq!(op(&mut a.member, &a.team, json!({ "op": "leave_group", "handle": "@design" })), Ok(Value::Null), "a leading @ is taken off");
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(json!([shown("design", "", &[owner.as_str()], None)])), "the owner is still in it");
    assert_eq!(op(&mut a.member, &a.team, json!({ "op": "leave_group", "handle": "design" })), Err("you are not in @design".into()));
    assert_eq!(op(&mut a.member, &a.team, json!({ "op": "join_group", "handle": "nobody" })), Err("there is no group @nobody".into()));
}

#[test]
fn a_fallback_that_is_not_a_member_is_refused() {
    let mut a = acme("groups_fallback");
    let owner = a.owner_id.clone();
    let member = a.member_id.clone();
    let stranger = a.stranger_id.clone();
    let both = sorted(vec![owner.clone(), member.clone()]);
    let mut asked = save("@design", "", &[owner.as_str(), member.as_str()]);
    asked["group"]["fallback"] = json!(stranger);
    assert_eq!(op(&mut a.owner, &a.team, asked.clone()), Err("the fallback must be a member of this team".into()));
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(json!([])), "nothing is stored");

    asked["group"]["fallback"] = json!(member.clone());
    assert_eq!(op(&mut a.owner, &a.team, asked), Ok(Value::Null), "a member may be the fallback");
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(json!([shown("design", "", &refs(&both), Some(member.as_str()))])));
}

/// The ids as `shown` takes them.
fn refs(ids: &[String]) -> Vec<&str> {
    ids.iter().map(String::as_str).collect()
}

#[test]
fn a_member_removed_from_the_team_leaves_every_group() {
    let mut a = acme("groups_removed");
    let owner = a.owner_id.clone();
    let member = a.member_id.clone();
    let mut design = save("@design", "", &[owner.as_str(), member.as_str()]);
    design["group"]["fallback"] = json!(member.clone());
    assert_eq!(op(&mut a.owner, &a.team, design), Ok(Value::Null));
    assert_eq!(op(&mut a.owner, &a.team, save("@ops", "", &[member.as_str()])), Ok(Value::Null));

    admin_on(&a.hub.database, &format!("delete from team_members where team = '{}' and member = '{member}'", a.team));

    assert_eq!(
        op(&mut a.owner, &a.team, json!({ "op": "groups" })),
        Ok(json!([shown("design", "", &[owner.as_str()], None), shown("ops", "", &[], None)])),
        "the removed member is in no group, and the fallback that was it reads as none"
    );
    assert_eq!(op(&mut a.member, &a.team, json!({ "op": "groups" })), Err(NOT_A_MEMBER.into()));
}

#[test]
fn a_stranger_gets_the_membership_error_on_every_group_op() {
    let mut a = acme("groups_stranger");
    let owner = a.owner_id.clone();
    assert_eq!(op(&mut a.owner, &a.team, save("@design", "", &[owner.as_str()])), Ok(Value::Null));

    assert_eq!(op(&mut a.stranger, &a.team, json!({ "op": "groups" })), Err(NOT_A_MEMBER.into()));
    assert_eq!(op(&mut a.stranger, &a.team, save("@ops", "", &[])), Err(NOT_A_MEMBER.into()));
    assert_eq!(op(&mut a.stranger, &a.team, json!({ "op": "join_group", "handle": "design" })), Err(NOT_A_MEMBER.into()));
    assert_eq!(op(&mut a.stranger, &a.team, json!({ "op": "leave_group", "handle": "design" })), Err(NOT_A_MEMBER.into()));
    assert_eq!(op(&mut a.owner, &a.team, json!({ "op": "groups" })), Ok(json!([shown("design", "", &[owner.as_str()], None)])), "the stranger made nothing");
}

#[test]
fn a_group_text_longer_than_its_limit_is_refused() {
    let mut a = acme("groups_text");
    let about = "a".repeat(281);
    assert_eq!(op(&mut a.owner, &a.team, save("@design", &about, &[])), Err("a group's description is at most 280 characters".into()));
    assert_eq!(op(&mut a.owner, &a.team, save("@design", &about[..280], &[])), Ok(Value::Null), "280 characters is the most");
    let mut slack = save("@design", "", &[]);
    slack["group"]["slack_group"] = json!("s".repeat(81));
    assert_eq!(op(&mut a.owner, &a.team, slack), Err("a group's Slack group is at most 80 characters".into()));
}
