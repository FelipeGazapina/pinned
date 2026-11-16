//! Teams: the guards on a team bot's calls. A step that hides a command in a secret is denied
//! without asking anyone. A Call step calls only an on bot of the same team.

use super::teams_calls::{calls_of, choice, design, on_node, op, patient, rows, TeamBox};
use super::*;
use pinned_core::preauth::team_hook::{self, HookEnv};
use pinned_core::teams::{Bot, BotLimits, CallRules, TeamOp};
use pinned_server::hub::Account;
use pinned_server::teams;
use uuid::Uuid;

const NOT_ON: &str = "the called workflow is not an on bot of this team";

/// A team that keeps the Upstash key and Claude token: ana owns it, bob is a member.
struct Crew {
    hub: TestHub,
    team: Uuid,
    ana: Arc<Account>,
    bob: Arc<Account>,
    folder: PathBuf,
}

impl Crew {
    fn new(name: &str) -> Crew {
        let hub = TestHub::start_with_team_policy(name, fast(), patient());
        let sign_up = |email: &str| {
            let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
            hub.runtime.block_on(hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap()
        };
        let (ana, bob) = (sign_up("ana@example.test"), sign_up("bob@example.test"));
        let made = hub.runtime.block_on(teams::call(hub.hub(), &ana, None, teams::Op::Team(TeamOp::Create { name: "Acme".into() }))).unwrap();
        let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{}', 'member')", bob.id));
        let folder = temp(&format!("teams-guards-{name}"));
        let crew = Crew { hub, team, ana, bob, folder };
        crew.call(&crew.ana, TeamOp::Hub { call: json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN }) }).unwrap();
        crew
    }

    fn call(&self, who: &Arc<Account>, op: TeamOp) -> Result<Value, String> {
        let op = teams::Op::read(op)?;
        self.hub.runtime.block_on(teams::call(self.hub.hub(), who, Some(self.team), op))
    }

    /// A workflow with a trigger watching `watch` and one `step` after it.
    fn graph(&self, name: &str, watch: Value, step: Value) -> Value {
        json!({
            "name": name, "enabled": false, "trigger": watch,
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
            "graph": {
                "nodes": [
                    { "id": "t", "name": "Trigger", "position": [0, 0], "type": "trigger", "watch": watch },
                    { "id": "s", "name": "Step", "position": [1, 0], "merge": step },
                ],
                "edges": [{ "from": "t", "port": "out", "to": "s" }],
            },
        })
    }

    /// The workflow bob saves with its bot: a manual trigger and one Call step to `callee`.
    fn caller(&self, callee: &str) -> Value {
        let mut caller = self.graph("Caller", json!({ "kind": "manual" }), json!({}));
        caller["graph"]["nodes"][1] = json!({ "id": "s", "name": "Call", "position": [1, 0], "type": "call", "workflow": callee });
        caller
    }

    /// A called workflow with one session step, saved by bob with its bot: a draft.
    fn callee(&self) -> String {
        let mut callee = self.graph("Helper", json!({ "kind": "called" }), json!({}));
        callee["graph"]["nodes"][1] = json!({ "id": "s", "name": "Fix", "position": [1, 0], "type": "session", "prompt": "Fix." });
        save_team_workflow(&self.hub, &self.bob, self.team, callee).unwrap()
    }

    fn turn_on(&self, id: &str, on: bool) -> Result<(), String> {
        self.call(&self.ana, TeamOp::TurnBotOn { workflow: id.to_owned(), on }).map(drop)
    }

    fn save_bot(&self, id: &str) -> Result<(), String> {
        let bot = Bot { workflow: id.to_owned(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] };
        self.call(&self.bob, TeamOp::SaveBot { bot, connector: None }).map(drop)
    }

    fn on_bots(&self) -> Vec<String> {
        let bots = self.call(&self.bob, TeamOp::Bots).unwrap();
        bots.as_array().unwrap().iter().filter(|b| b["on"] == json!(true)).map(|b| b["workflow"].as_str().unwrap().to_owned()).collect()
    }
}

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

#[test]
fn a_call_to_a_draft_callee_is_refused_at_save_and_turn_on() {
    let crew = Crew::new("teams_guards_callee_draft");
    let callee = crew.callee();
    assert_eq!(save_team_workflow(&crew.hub, &crew.bob, crew.team, crew.caller(&callee)).map(drop), Err(NOT_ON.to_owned()), "saving the caller");
    assert_eq!(team_workflows(&crew.hub, crew.team).len(), 1, "only the callee is stored");
    let caller = store_on_team(&crew.hub, crew.team, crew.caller(&callee));
    assert_eq!(crew.save_bot(&caller), Err(NOT_ON.to_owned()), "saving the caller's bot");
    assert_eq!(crew.turn_on(&caller, true), Err(NOT_ON.to_owned()), "turning the caller on");
    // Once the callee is on, the caller saves and turns on.
    crew.turn_on(&callee, true).unwrap();
    assert_eq!(crew.save_bot(&caller), Ok(()));
    assert_eq!(crew.turn_on(&caller, true), Ok(()));
    let mut on = crew.on_bots();
    on.sort();
    let mut both = vec![callee, caller];
    both.sort();
    assert_eq!(on, both);
}

#[test]
fn a_call_to_an_off_callee_is_refused_at_turn_on() {
    let crew = Crew::new("teams_guards_callee_off");
    let callee = crew.callee();
    crew.turn_on(&callee, true).unwrap();
    let caller = save_team_workflow(&crew.hub, &crew.bob, crew.team, crew.caller(&callee)).unwrap();
    crew.turn_on(&callee, false).unwrap();
    assert_eq!(crew.turn_on(&caller, true), Err(NOT_ON.to_owned()));
    assert_eq!(crew.on_bots(), Vec::<String>::new());
}

#[test]
fn a_call_to_a_deleted_callee_is_refused_at_turn_on() {
    let crew = Crew::new("teams_guards_callee_deleted");
    let callee = crew.callee();
    crew.turn_on(&callee, true).unwrap();
    let caller = save_team_workflow(&crew.hub, &crew.bob, crew.team, crew.caller(&callee)).unwrap();
    crew.turn_on(&callee, false).unwrap();
    crew.call(&crew.ana, TeamOp::DeleteBot { workflow: callee.clone() }).unwrap();
    assert_eq!(crew.turn_on(&caller, true), Err(NOT_ON.to_owned()));
    assert_eq!(crew.on_bots(), Vec::<String>::new());
}

#[test]
fn a_call_to_a_personal_workflow_is_refused_at_save() {
    let crew = Crew::new("teams_guards_callee_personal");
    let mut mine = crew.graph("Mine", json!({ "kind": "called" }), json!({}));
    mine["graph"]["nodes"][1] = json!({ "id": "s", "name": "Fix", "position": [1, 0], "type": "session", "prompt": "Fix." });
    mine["enabled"] = json!(true);
    let saved = crew.hub.runtime.block_on(crew.hub.hub().hub_call(&crew.bob, HubCall::SaveConnector { connector: serde_json::from_value(mine).unwrap() })).unwrap();
    let personal = saved["id"].as_str().unwrap().to_owned();
    assert_eq!(save_team_workflow(&crew.hub, &crew.bob, crew.team, crew.caller(&personal)).map(drop), Err(NOT_ON.to_owned()));
    assert_eq!(team_workflows(&crew.hub, crew.team), Vec::<Value>::new(), "nothing is stored");
}

#[test]
fn a_callee_turned_off_after_the_caller_went_on_stops_its_run_before_anything_outside() {
    let crew = Crew::new("teams_guards_callee_run");
    let callee = crew.callee();
    crew.turn_on(&callee, true).unwrap();
    let caller = save_team_workflow(&crew.hub, &crew.bob, crew.team, crew.caller(&callee)).unwrap();
    crew.turn_on(&caller, true).unwrap();
    crew.turn_on(&callee, false).unwrap();
    assert_eq!(crew.on_bots(), vec![caller.clone()], "the caller stays on");
    let ran = crew.call(&crew.bob, TeamOp::RunNow { workflow: caller.clone() });
    assert_eq!(ran, Err(NOT_ON.to_owned()));
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new(), "no box opens for the callee");
    assert_eq!(crew.call(&crew.bob, TeamOp::Runs).unwrap(), json!([]), "no run is recorded");
}

#[test]
fn a_teams_owner_may_not_set_one_box_reused() {
    let crew = Crew::new("teams_guards_fixed_refused");
    for kind in ["workflow", "session", "worker", "orchestrator"] {
        let call = json!({ "cmd": "set_box_access", "access": { kind: { "mode": "fixed" } } });
        assert_eq!(crew.call(&crew.ana, TeamOp::Hub { call }).map(drop), Err("a team's runs each get a box of their own".to_owned()), "{kind}");
    }
    let ask = json!({ "cmd": "set_box_access", "access": { "workflow": { "mode": "ask" } } });
    assert!(crew.call(&crew.ana, TeamOp::Hub { call: ask }).is_ok(), "other kinds stay the owner's to set");
    let fresh = json!({ "cmd": "set_box_access", "access": { "workflow": { "mode": "fresh" } } });
    assert!(crew.call(&crew.ana, TeamOp::Hub { call: fresh }).is_ok());
}

#[test]
fn a_team_with_one_box_reused_stored_runs_on_a_fresh_box() {
    let mut crew = Crew::new("teams_guards_fixed_stored");
    let mut solo = crew.graph("Solo", json!({ "kind": "manual" }), json!({}));
    solo["graph"]["nodes"][1] = json!({ "id": "s", "name": "Fix", "position": [1, 0], "type": "session", "prompt": "Fix." });
    let solo = save_team_workflow(&crew.hub, &crew.bob, crew.team, solo).unwrap();
    crew.turn_on(&solo, true).unwrap();
    // An older hub let the owner store "One box, reused" on the team.
    let addr = crew.hub.stop();
    admin_on(&crew.hub.database, &format!("update accounts set store = jsonb_set(store, '{{boxes}}', '{{\"workflow\":{{\"mode\":\"fixed\"}}}}') where id = '{}'", crew.team));
    crew.hub.serve(&addr);
    let account = |id: Uuid| crew.hub.runtime.block_on(crew.hub.hub().account(id)).unwrap();
    (crew.ana, crew.bob) = (account(crew.ana.id), account(crew.bob.id));
    assert_eq!(team_settings(&crew.hub, crew.team)["boxes"]["workflow"], json!({ "mode": "fixed" }));

    crew.call(&crew.bob, TeamOp::RunNow { workflow: solo }).unwrap();
    assert_eq!(crew.hub.boxes.live().len(), 1, "the run opened a box");
    assert_eq!(team_settings(&crew.hub, crew.team)["boxes"]["workflow"], json!({ "mode": "fixed" }), "no shared box was claimed");
}

#[test]
fn an_unused_go_ahead_does_not_survive_a_take_over() {
    const DEPLOY: &str = "vercel deploy --prod site-a";
    let hub = TestHub::start_with_team_policy("teams_guards_go_ahead_take_over", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut bea = tb.member(&hub, "bea@example.test", &[]);
    tb.bot(&hub, design(), Some(bea.id));
    let run = tb.run(&hub);
    assert!(try_step(&hub, &tb, DEPLOY).unwrap().starts_with("Asked the team before this step"));
    let first = calls_of(tb.team, &mut tb.owner.client);
    assert_eq!(first.len(), 1);
    let call = first[0]["id"].as_str().unwrap().to_owned();
    op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": choice("Go ahead", "") })).unwrap();

    // bea, the bot's owner, takes over before the bot used the owner's Go ahead.
    op(tb.team, &mut bea.client, json!({ "op": "take_over", "run": run.to_string() })).unwrap();
    let reason = try_step(&hub, &tb, DEPLOY).expect("the step is asked about again, not run");
    let listed = calls_of(tb.team, &mut tb.owner.client);
    assert_eq!(listed.len(), 2, "a new call is listed");
    let again = listed.iter().map(|c| c["id"].as_str().unwrap().to_owned()).find(|id| *id != call).unwrap();
    assert_eq!(reason, format!("Asked the team before this step (call {again}). End your turn and wait for the answer; only if it says go ahead, make this exact tool call again, unchanged."));
    let open = rows(&hub, &format!("select (answer is null and released_at is null)::text from team_calls where id = '{again}'"));
    assert_eq!(open[0][0].as_deref(), Some("true"));
}
