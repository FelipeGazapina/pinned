//! Teams: the guards on a team bot's calls. A step that hides a command in a secret is denied
//! without asking anyone.

use super::teams_calls::{calls_of, on_node, patient, TeamBox};
use super::*;
use pinned_core::preauth::team_hook::{self, HookEnv};

/// What the bot reads when it runs `command` in the team's box: `None` when the step runs, else
/// the hook's reason for denying it.
fn try_step(hub: &TestHub, tb: &TeamBox, command: &str) -> Option<String> {
    let payload = json!({ "tool_name": "Bash", "tool_input": { "command": command }, "cwd": "/work/site" }).to_string();
    let env = HookEnv { team_box: true, project_dir: Some(PathBuf::from("/work/site")), home: PathBuf::from("/home/box") };
    let out = on_node(hub, &tb.box_id, |node| team_hook::decide(&payload, &env, |m| node.link.as_ref().unwrap().forward(m)))?;
    let out: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    Some(out["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().to_owned())
}

#[test]
fn a_step_hiding_a_command_in_a_secret_is_denied_and_no_call_is_opened() {
    let hub = TestHub::start_with_team_policy("teams_guards_hidden_command", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut member = tb.member(&hub, "member@example.test", &["ops"]);
    tb.run(&hub);
    for command in ["SECRET_KEY=$(git push -f origin main) vercel deploy --prod", "API_TOKEN=`git push -f origin main` vercel deploy --prod"] {
        assert_eq!(try_step(&hub, &tb, command).as_deref(), Some("this step hides a command in a secret"), "{command}");
    }
    assert_eq!(calls_of(tb.team, &mut tb.owner.client), Vec::<Value>::new(), "the owner is asked nothing");
    assert_eq!(calls_of(tb.team, &mut member.client), Vec::<Value>::new(), "a member is asked nothing");
    // A plain literal secret is still hidden, and the step goes to the team.
    assert!(try_step(&hub, &tb, "SECRET_KEY=abc123 vercel deploy --prod").unwrap().starts_with("Asked the team before this step"));
    let listed = calls_of(tb.team, &mut tb.owner.client);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["question"], "deploy: SECRET_KEY=[hidden] vercel deploy --prod");
}
