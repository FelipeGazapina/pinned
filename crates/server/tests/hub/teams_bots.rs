//! pinned Teams, bots: the draft/on rule, boxes forced, and a run of a bot on the team's own keys.

use super::*;
use pinned_core::teams::{Bot, BotLimits, CallRules, Run as TeamRun, RunState, TeamOp, Who};
use pinned_server::hub::Account;
use pinned_server::teams;
use uuid::Uuid;

const DRAFT: &str = "this bot is a draft: it has to be turned on before it can run";
const NEEDS_KEYS: &str = "the team needs its Upstash key and Claude token before a bot can run";
const NOT_ASKED: &str = "only the team's owner, or someone this bot asks, turns it on";
const MEMBER_TOKEN: &str = "sk-ant-oat01-membertoken";

/// A hub with a team: ana owns it; bob, carl and dan are members.
struct Crew {
    hub: TestHub,
    team: Uuid,
    ana: Arc<Account>,
    bob: Arc<Account>,
    carl: Arc<Account>,
    dan: Arc<Account>,
    folder: PathBuf,
}

impl Crew {
    fn new(name: &str) -> Crew {
        let hub = TestHub::start_with_boxes(name, Signup::Open, fast());
        let sign_up = |email: &str| {
            let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
            hub.runtime.block_on(hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap()
        };
        let (ana, bob, carl, dan) = (sign_up("ana@example.test"), sign_up("bob@example.test"), sign_up("carl@example.test"), sign_up("dan@example.test"));
        let made = hub.runtime.block_on(teams::call(hub.hub(), &ana, None, teams::Op::Team(TeamOp::Create { name: "Acme".into() }))).unwrap();
        let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        for member in [&bob, &carl, &dan] {
            admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{}', 'member')", member.id));
        }
        let folder = temp(&format!("teams-bots-{name}"));
        Crew { hub, team, ana, bob, carl, dan, folder }
    }

    fn call(&self, who: &Arc<Account>, op: TeamOp) -> Result<Value, String> {
        let op = teams::Op::read(op)?;
        self.hub.runtime.block_on(teams::call(self.hub.hub(), who, Some(self.team), op))
    }

    fn hub_op(&self, who: &Arc<Account>, call: Value) -> Result<Value, String> {
        self.call(who, TeamOp::Hub { call })
    }

    /// The owner gives the team its Upstash key and Claude token.
    fn give_keys(&self) {
        let connected = self.hub_op(&self.ana, json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN })).unwrap();
        assert_eq!(connected, json!({ "upstash": true, "claude": true, "github": false, "slack": false, "linear": false }), "which keys are set, never a hint");
    }

    fn connector(&self, name: &str, machine: Value) -> Value {
        json!({
            "name": name, "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": machine,
        })
    }

    /// A workflow in the team's store, saved by `who` with its bot (`TeamOp::SaveBot`); answers
    /// what the hub stored.
    fn save_workflow(&self, who: &Arc<Account>, connector: Value) -> Result<Value, String> {
        let id = save_team_workflow(&self.hub, who, self.team, connector)?;
        Ok(self.stored(&id).expect("stored"))
    }

    fn workflow(&self, name: &str) -> String {
        let saved = self.save_workflow(&self.bob, self.connector(name, json!({ "kind": "upstash-box", "size": "small" }))).unwrap();
        saved["id"].as_str().unwrap().to_owned()
    }

    /// The stored workflow `id` as the team's settings list it.
    fn stored(&self, id: &str) -> Option<Value> {
        team_workflows(&self.hub, self.team).into_iter().find(|c| c["id"] == id)
    }

    fn bots(&self) -> Vec<Bot> {
        serde_json::from_value(self.call(&self.carl, TeamOp::Bots).unwrap()).unwrap()
    }

    fn bot(&self, id: &str) -> Bot {
        self.bots().into_iter().find(|b| b.workflow == id).unwrap_or_else(|| panic!("no bot {id}"))
    }

    fn save_bot(&self, who: &Arc<Account>, bot: &Bot) -> Result<Value, String> {
        self.call(who, TeamOp::SaveBot { bot: bot.clone(), connector: None })
    }

    fn turn_on(&self, who: &Arc<Account>, id: &str, on: bool) -> Result<Value, String> {
        self.call(who, TeamOp::TurnBotOn { workflow: id.into(), on })
    }

    fn runs(&self) -> Vec<TeamRun> {
        self.hub.runtime.block_on(teams::db::runs(&self.hub.hub().db, self.team, 50)).unwrap()
    }

    fn group(&self, handle: &str, members: &[&Arc<Account>]) {
        admin_on(&self.hub.database, &format!("insert into team_groups (team, handle) values ('{}', '{handle}')", self.team));
        for member in members {
            admin_on(&self.hub.database, &format!("insert into team_group_members (team, handle, member) values ('{}', '{handle}', '{}')", self.team, member.id));
        }
    }
}

fn bot_of(workflow: &str) -> Bot {
    Bot { workflow: workflow.into(), name: String::new(), owner: None, calls: CallRules::default(), limits: BotLimits { usd: 5.0, minutes: 30 }, on: false, on_by: None, once: false, off_note: None, skipped: vec![] }
}

/// A bot that is on, made the way a team does: keys, a saved bot, the owner's yes.
fn on_bot(crew: &Crew, name: &str) -> String {
    crew.give_keys();
    let id = crew.workflow(name);
    crew.save_bot(&crew.bob, &bot_of(&id)).unwrap();
    crew.turn_on(&crew.ana, &id, true).unwrap();
    assert!(crew.bot(&id).on);
    id
}

#[test]
fn a_draft_bot_cannot_run_by_run_now_or_run_workflow() {
    let crew = Crew::new("tb_draft");
    crew.give_keys();
    let id = crew.workflow("Fix bugs");
    let bare = crew.call(&crew.bob, TeamOp::RunNow { workflow: id.clone() });
    assert_eq!(bare.unwrap_err(), DRAFT, "a workflow saved with its bot is a draft");
    crew.save_bot(&crew.bob, &bot_of(&id)).unwrap();
    assert!(!crew.bot(&id).on);
    assert_eq!(crew.call(&crew.bob, TeamOp::RunNow { workflow: id.clone() }).unwrap_err(), DRAFT);
    assert_eq!(crew.hub_op(&crew.bob, json!({ "cmd": "run_workflow", "id": id })).unwrap_err(), "run_workflow does not run on a team");
    assert_eq!(crew.runs(), vec![], "nothing started");
    assert_eq!(crew.hub.boxes.live(), Vec::<String>::new());
}

#[test]
fn only_the_owner_or_someone_the_bot_asks_turns_it_on() {
    let crew = Crew::new("tb_turn_on");
    crew.give_keys();
    crew.group("design", &[&crew.bob]);
    let id = crew.workflow("Fix bugs");
    let mut bot = bot_of(&id);
    bot.calls.in_doubt = Who::Group { handle: "design".into() };
    bot.calls.on_failure = Who::Person { account: crew.dan.id.to_string() };
    crew.save_bot(&crew.carl, &bot).unwrap();

    assert_eq!(crew.turn_on(&crew.carl, &id, true).unwrap_err(), NOT_ASKED);
    assert!(!crew.bot(&id).on);
    assert_eq!(crew.turn_on(&crew.bob, &id, true), Ok(Value::Null), "a member of the called group");
    let on = crew.bot(&id);
    assert_eq!((on.on, on.on_by), (true, Some(crew.bob.id.to_string())));
    assert_eq!(crew.stored(&id).unwrap()["enabled"], true, "its workflow is enabled with it");

    assert_eq!(crew.turn_on(&crew.carl, &id, false), Ok(Value::Null), "any member turns it off");
    assert_eq!((crew.bot(&id).on, crew.bot(&id).on_by), (false, None));
    assert_eq!(crew.stored(&id).unwrap()["enabled"], false);

    assert_eq!(crew.turn_on(&crew.dan, &id, true), Ok(Value::Null), "the person a rule names");
    assert_eq!(crew.bot(&id).on_by, Some(crew.dan.id.to_string()));
    crew.turn_on(&crew.carl, &id, false).unwrap();
    assert_eq!(crew.turn_on(&crew.ana, &id, true), Ok(Value::Null), "the owner");
    assert_eq!(crew.bot(&id).on_by, Some(crew.ana.id.to_string()));
}

#[test]
fn editing_an_on_bot_turns_it_off_by_save_bot_and_by_save_connector() {
    let crew = Crew::new("tb_edit");
    let id = on_bot(&crew, "Fix bugs");
    assert_eq!(crew.stored(&id).unwrap()["enabled"], true);

    let mut bot = bot_of(&id);
    bot.calls.in_doubt = Who::Owner;
    crew.save_bot(&crew.carl, &bot).unwrap();
    let after = crew.bot(&id);
    assert_eq!((after.on, after.on_by, after.owner), (false, None, Some(crew.bob.id.to_string())), "off, and the bot keeps the member who made it");
    assert_eq!(crew.stored(&id).unwrap()["enabled"], false, "its workflow is a draft too");

    crew.turn_on(&crew.ana, &id, true).unwrap();
    let mut connector = crew.stored(&id).unwrap();
    connector["instructions"] = json!("Fix the bug and open a pull request.");
    connector["enabled"] = json!(true);
    let saved = crew.save_workflow(&crew.carl, connector).unwrap();
    assert_eq!(saved["instructions"], "Fix the bug and open a pull request.");
    assert_eq!(saved["enabled"], false);
    assert_eq!(crew.bot(&id).on, false, "a changed workflow is a draft until someone turns it on");
    assert_eq!(crew.call(&crew.carl, TeamOp::RunNow { workflow: id }).unwrap_err(), DRAFT);
}

#[test]
fn a_member_cannot_enable_a_workflow_by_saving_it_or_run_it_on_their_machine() {
    let crew = Crew::new("tb_fresh");
    let mut connector = crew.connector("Fix bugs", json!(""));
    connector["enabled"] = json!(true);
    let saved = crew.save_workflow(&crew.bob, connector).unwrap();
    assert_eq!(saved["machine"], json!({ "kind": "upstash-box" }), "RunsOn::Any is stored as a box");
    assert_eq!(saved["enabled"], false, "a new workflow is saved off, whatever the member asked");
    let id = saved["id"].as_str().unwrap();
    assert_eq!(crew.stored(id).unwrap()["machine"], json!({ "kind": "upstash-box" }));

    let mut named = crew.stored(id).unwrap();
    named["machine"] = json!("machine-of-bob");
    assert_eq!(crew.save_workflow(&crew.bob, named).unwrap()["machine"], json!({ "kind": "upstash-box" }), "a member's own machine too");
    let mut sized = crew.stored(id).unwrap();
    sized["machine"] = json!({ "kind": "upstash-box", "size": "large" });
    assert_eq!(crew.save_workflow(&crew.bob, sized).unwrap()["machine"], json!({ "kind": "upstash-box", "size": "large" }), "a box size is kept");
}

#[test]
fn turning_a_bot_on_needs_the_teams_upstash_key_and_claude_token() {
    let crew = Crew::new("tb_keys");
    let id = crew.workflow("Fix bugs");
    crew.save_bot(&crew.bob, &bot_of(&id)).unwrap();
    assert_eq!(crew.turn_on(&crew.ana, &id, true).unwrap_err(), NEEDS_KEYS);
    assert_eq!(crew.stored(&id).unwrap()["enabled"], false);
    assert!(!crew.bot(&id).on);
    crew.give_keys();
    assert_eq!(crew.turn_on(&crew.ana, &id, true), Ok(Value::Null));
    assert!(crew.bot(&id).on);
}

#[test]
fn a_run_now_opens_a_box_on_the_teams_claude_token_not_a_members() {
    let crew = Crew::new("tb_run");
    // Bob has an Upstash box of his own on a different Claude token, on his personal account.
    let own = serde_json::from_value::<HubCall>(json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": MEMBER_TOKEN })).unwrap();
    crew.hub.runtime.block_on(crew.hub.hub().hub_call(&crew.bob, own)).unwrap();
    let id = on_bot(&crew, "Fix bugs");

    let run = serde_json::from_value::<TeamRun>(crew.call(&crew.carl, TeamOp::RunNow { workflow: id.clone() }).unwrap()).unwrap();
    assert_eq!((run.workflow.as_str(), run.state, run.started_by.as_str()), (id.as_str(), RunState::Working, crew.carl.id.to_string().as_str()));
    let listed = crew.runs();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].id.as_str(), listed[0].state, listed[0].started_by.as_str()), (run.id.as_str(), RunState::Working, crew.carl.id.to_string().as_str()));

    wait_for("the box's sign-in", || box_tokens(&crew.hub.boxes).len() == 1);
    assert_eq!(box_tokens(&crew.hub.boxes), vec![CLAUDE_TOKEN], "the team's token, not {MEMBER_TOKEN}");
    assert_eq!(crew.hub.runtime.block_on(crew.hub.hub().db.boxes(crew.team)).unwrap().len(), 1, "the box is the team's");
    assert_eq!(crew.hub.runtime.block_on(crew.hub.hub().db.boxes(crew.bob.id)).unwrap().len(), 0, "and none is bob's");

    // The personal door does not run a team's workflow: Run now is the one way.
    assert_eq!(crew.hub_op(&crew.bob, json!({ "cmd": "run_workflow", "id": id })).unwrap_err(), "run_workflow does not run on a team");
    // Run now again, while that run is live, joins the bot's line behind it.
    let again = crew.call(&crew.carl, TeamOp::RunNow { workflow: id.clone() }).unwrap();
    assert_eq!((again["workflow"].as_str(), again["waiting"].as_bool(), again["place"].as_i64()), (Some(id.as_str()), Some(true), Some(1)));
    assert_eq!(crew.runs().len(), 1);
}

#[test]
fn a_once_bot_is_discarded_when_its_run_ends() {
    let crew = Crew::new("tb_once");
    let (keep, once) = (crew.workflow("Keep"), crew.workflow("Once"));
    crew.save_bot(&crew.bob, &bot_of(&keep)).unwrap();
    crew.save_bot(&crew.bob, &Bot { once: true, ..bot_of(&once) }).unwrap();
    assert_eq!(crew.bots().iter().map(|b| (b.name.as_str(), b.once)).collect::<Vec<_>>(), vec![("Keep", false), ("Once", true)]);

    let ended = |workflow: &str, state: RunState| TeamRun { id: Uuid::new_v4().to_string(), workflow: workflow.into(), state, controller: None, cost_usd: 0.0, model_usd: Some(0.0), active_secs: 0, waiting_secs: 0, started_at: 1, ended_at: Some(2), outcome: None, started_by: crew.bob.id.to_string() };
    let ended_run = |workflow: &str, state: RunState| crew.hub.runtime.block_on(teams::bots::run_ended(crew.hub.hub(), crew.team, &ended(workflow, state))).unwrap();
    ended_run(&once, RunState::Working);
    assert_eq!(crew.bots().len(), 2, "a run that has not ended discards nothing");
    ended_run(&keep, RunState::Done);
    assert_eq!(crew.bots().len(), 2, "a bot that was not made to run once stays");
    ended_run(&once, RunState::Failed);
    assert_eq!(crew.bots().iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), vec!["Keep"]);
    assert_eq!(crew.stored(&once), None, "its workflow went with it");
}

#[test]
fn limits_above_the_policy_ceiling_are_the_owners_to_give() {
    let crew = Crew::new("tb_limits");
    let id = crew.workflow("Fix bugs");
    let big = Bot { limits: BotLimits { usd: 20.0, minutes: 120 }, ..bot_of(&id) };
    assert_eq!(
        crew.save_bot(&crew.bob, &big).unwrap_err(),
        "a bot may spend at most $5 and run 30 minutes unless the team's owner sets more"
    );
    assert_eq!(crew.bot(&id).limits, BotLimits { usd: 5.0, minutes: 30 });
    assert_eq!(crew.save_bot(&crew.ana, &big), Ok(json!({ "workflow": id })));
    assert_eq!(crew.bot(&id).limits, BotLimits { usd: 20.0, minutes: 120 });
    let rules = Bot { calls: CallRules { in_doubt: Who::Owner, ..CallRules::default() }, ..big.clone() };
    assert_eq!(crew.save_bot(&crew.bob, &rules), Ok(json!({ "workflow": id })), "a member keeps what the owner gave");
    let more = Bot { limits: BotLimits { usd: 30.0, minutes: 120 }, ..big };
    assert!(crew.save_bot(&crew.bob, &more).is_err(), "but does not raise it");
    let none = Bot { limits: BotLimits { usd: 0.0, minutes: 10 }, ..bot_of(&id) };
    assert_eq!(crew.save_bot(&crew.ana, &none).unwrap_err(), "a run needs a limit above 0 dollars and 0 minutes");
}

#[test]
fn a_bot_asks_only_members_and_groups_of_the_team() {
    let crew = Crew::new("tb_who");
    let id = crew.workflow("Fix bugs");
    let stranger = Uuid::new_v4().to_string();
    let person = Bot { calls: CallRules { in_doubt: Who::Person { account: stranger.clone() }, ..CallRules::default() }, ..bot_of(&id) };
    assert_eq!(crew.save_bot(&crew.bob, &person).unwrap_err(), format!("{stranger} is not a member of this team"));
    let group = Bot { calls: CallRules { on_limit: Who::Group { handle: "ghosts".into() }, ..CallRules::default() }, ..bot_of(&id) };
    assert_eq!(crew.save_bot(&crew.bob, &group).unwrap_err(), "this team has no group @ghosts");
    assert_eq!(crew.save_bot(&crew.bob, &bot_of("c0-nope")).unwrap_err(), "this team has no workflow c0-nope");
}

#[test]
fn deleting_the_workflow_removes_the_bot() {
    let crew = Crew::new("tb_delete");
    let id = crew.workflow("Fix bugs");
    crew.save_bot(&crew.bob, &bot_of(&id)).unwrap();
    assert_eq!(crew.bots().len(), 1);
    assert_eq!(crew.hub_op(&crew.carl, json!({ "cmd": "delete_connector", "id": id })).unwrap_err(), "delete_connector does not run on a team");
    assert_eq!(crew.bots().len(), 1, "the personal door deletes nothing");
    crew.call(&crew.bob, TeamOp::DeleteBot { workflow: id.clone() }).unwrap();
    assert_eq!(crew.bots(), vec![]);
    assert_eq!(crew.hub.runtime.block_on(teams::db::bots(&crew.hub.hub().db, crew.team)).unwrap(), vec![], "and its row");
}

#[test]
fn a_member_reads_a_bots_workflow_and_another_team_reads_none() {
    let crew = Crew::new("tb_workflow");
    let id = crew.workflow("Fix bugs");
    crew.save_bot(&crew.bob, &bot_of(&id)).unwrap();
    let workflow = crew.call(&crew.carl, TeamOp::BotWorkflow { workflow: id.clone() }).unwrap();
    assert_eq!(workflow["id"], id);
    assert_eq!(workflow["name"], "Fix bugs");
    assert_eq!(
        crew.call(&crew.carl, TeamOp::BotWorkflow { workflow: "no-such".into() }).unwrap_err(),
        "this team has no workflow no-such"
    );
    let (_, view) = account::signup(&crew.hub.url, "eve@example.test", "correct horse").unwrap();
    let eve = crew.hub.runtime.block_on(crew.hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap();
    let other = teams::Op::Team(TeamOp::Create { name: "Other".into() });
    let other_team = Uuid::parse_str(crew.hub.runtime.block_on(teams::call(crew.hub.hub(), &eve, None, other)).unwrap()["id"].as_str().unwrap()).unwrap();
    let read = teams::Op::read(TeamOp::BotWorkflow { workflow: id.clone() }).unwrap();
    assert_eq!(
        crew.hub.runtime.block_on(teams::call(crew.hub.hub(), &eve, Some(other_team), read)).unwrap_err(),
        format!("this team has no workflow {id}"),
        "a member of another team reads none of this team's workflows by id"
    );
    let outside = teams::Op::read(TeamOp::BotWorkflow { workflow: id }).unwrap();
    assert_eq!(
        crew.hub.runtime.block_on(teams::call(crew.hub.hub(), &eve, Some(crew.team), outside)).unwrap_err(),
        "you are not a member of that team",
        "a person on no team of it reads none"
    );
}
