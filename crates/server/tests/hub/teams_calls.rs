//! Teams: a bot in a team box asks people, the first answer wins, an unanswered call escalates
//! to the owner and then gives up, and a person takes over, gives back or stops the run.
//!
//! Members, groups, bots and runs are written with SQL (and `teams::db::insert_run`): their own
//! ops belong to other units and may still be stubs here.

use super::*;
use pinned_core::teams::{Run as TeamRun, RunState, Situation, TeamOp, TeamPolicy};
use pinned_server::teams;
use uuid::Uuid;

/// Long enough that no call escalates or gives up while a test that does not wait for it runs.
pub(super) fn patient() -> TeamPolicy {
    TeamPolicy { escalate_after_secs: 3600, give_up_after_secs: 7200, ..TeamPolicy::default() }
}

pub(super) struct Person {
    pub(super) id: Uuid,
    pub(super) client: Client,
}

pub(super) fn person(hub: &TestHub, email: &str) -> Person {
    let (token, account) = account::signup(&hub.url, email, "correct horse").unwrap();
    let mut client = Client::connect(&hub.url, &token);
    client.wait("hello", |f| f["type"] == "hello");
    Person { id: Uuid::parse_str(&account.id).unwrap(), client }
}

/// Runs SQL on the hub's database, as the teams' other ops would write it.
pub(super) fn sql(hub: &TestHub, statement: &str) {
    hub.runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        db.batch_execute(statement).await.unwrap();
    })
}

/// Each row of `query`, its columns as text.
pub(super) fn rows(hub: &TestHub, query: &str) -> Vec<Vec<Option<String>>> {
    hub.runtime.block_on(async {
        let (db, connection) = tokio_postgres::connect(&format!("{}/{}", postgres(), hub.database), tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(connection);
        let rows = db.query(query, &[]).await.unwrap();
        rows.iter().map(|r| (0..r.len()).map(|i| r.get::<_, Option<String>>(i)).collect()).collect()
    })
}

/// A team whose owner connected Upstash and keeps a box workflow, with one box booted for it.
pub(super) struct TeamBox {
    pub(super) owner: Person,
    pub(super) team: Uuid,
    pub(super) workflow: String,
    pub(super) machine: String,
    pub(super) box_id: String,
    /// The box node's folder, where its socket is.
    pub(super) dir: PathBuf,
    pub(super) tab: String,
}

impl TeamBox {
    pub(super) fn new(hub: &TestHub) -> TeamBox {
        let mut owner = person(hub, "olga@example.test");
        let made = owner.client.hub(json!({ "cmd": "team", "op": { "op": "create", "name": "Acme" } })).unwrap();
        let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        let mut on_team = |call: Value| owner.client.hub(json!({ "cmd": "team", "team": team.to_string(), "op": { "op": "hub", "call": call } }));
        on_team(json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN })).unwrap();
        let repo = temp(&format!("team-repo-{}", hub.database));
        let saved = on_team(json!({ "cmd": "save_connector", "connector": {
            "name": "Footer", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": repo, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": "", "machine": { "kind": "upstash-box", "size": "small" },
        } }))
        .unwrap();
        let workflow = saved["id"].as_str().unwrap().to_owned();
        let run = Run {
            cwd: repo,
            repo: None,
            args: vec!["--name".into(), "Fix the footer".into()],
            connector: workflow.clone(),
            subject: issue("team-1"),
            mentioned: vec![],
            account: None,
            runs_on: RunsOn::FreshBox(BoxStep { size: Some(BoxSize::Small) }),
            input: String::new(),
        };
        // A team's box opens only for a run of its bot: the bot is turned on and its trigger fires.
        // Its run and bot rows are cleared again, as each test writes the ones it needs.
        owner.client.hub(json!({ "cmd": "team", "team": team.to_string(), "op": { "op": "turn_bot_on", "workflow": workflow, "on": true } })).unwrap();
        hub.runtime.block_on(teams::triggers::fired(hub.hub(), team, Work::Run(run))).unwrap();
        sql(hub, &format!("delete from team_runs where team = '{team}'; delete from team_bots where team = '{team}'"));
        let row = hub.runtime.block_on(hub.hub().db.boxes(team)).unwrap().into_iter().next().unwrap().unwrap();
        let (machine, box_id) = (row.machine.to_string(), row.upstash.unwrap().0);
        let dir = hub.boxes.root.join(&box_id);
        let mut tab = String::new();
        wait_for("the box's session", || {
            let tabs = hub.runtime.block_on(hub.hub().account(team)).unwrap().tabs_of(&machine).unwrap_or_default();
            tab = tabs.first().map(|t| t.id.clone()).unwrap_or_default();
            !tab.is_empty()
        });
        on_node(hub, &box_id, |node| node.machine.listen(|_| Err("unexpected".into())).unwrap());
        TeamBox { owner, team, workflow, machine, box_id, dir, tab }
    }

    /// A member of the team, in the groups `groups`.
    pub(super) fn member(&self, hub: &TestHub, email: &str, groups: &[&str]) -> Person {
        let member = person(hub, email);
        sql(hub, &format!("insert into team_members (team, member, role) values ('{}', '{}', 'member')", self.team, member.id));
        for group in groups {
            sql(hub, &format!(
                "insert into team_groups (team, handle) values ('{0}', '{group}') on conflict do nothing;
                 insert into team_group_members (team, handle, member) values ('{0}', '{group}', '{1}')",
                self.team, member.id
            ));
        }
        member
    }

    /// The bot of the box's workflow, asking `in_doubt` and answered for by `owner`.
    pub(super) fn bot(&self, hub: &TestHub, in_doubt: Value, owner: Option<Uuid>) {
        let owner = owner.map_or("null".to_owned(), |o| format!("'{o}'"));
        let calls = json!({ "in_doubt": in_doubt });
        sql(hub, &format!(
            "insert into team_bots (team, workflow, owner, calls, limits) values ('{}', '{}', {owner}, '{calls}', '{{\"usd\":5.0,\"minutes\":30}}')",
            self.team, self.workflow
        ));
    }

    /// A run of the bot working in this box.
    pub(super) fn run(&self, hub: &TestHub) -> Uuid {
        let id = Uuid::new_v4();
        let run = TeamRun {
            id: id.to_string(),
            workflow: self.workflow.clone(),
            state: RunState::Working,
            controller: None,
            cost_usd: 0.0,
            active_secs: 0,
            started_at: 0,
            ended_at: None,
            outcome: None,
            started_by: self.owner.id.to_string(),
        };
        hub.runtime.block_on(teams::db::insert_run(&hub.hub().db, self.team, &run, Some(&self.machine))).unwrap();
        id
    }

    /// What the box's session sends the hub with `ask_people`.
    fn ask(&self, hub: &TestHub, question: &str, choices: &[&str]) -> Result<Value, String> {
        ask_from(hub, &self.box_id, question, choices)
    }

    /// The op run by account `who` straight on the hub, for two at once.
    async fn op_of(&self, hub: &Arc<pinned_server::hub::Hub>, who: Uuid, op: Value) -> Result<Value, String> {
        let person = hub.account(who).await?;
        teams::call(hub, &person, Some(self.team), teams::Op::read(serde_json::from_value::<TeamOp>(op).unwrap())?).await
    }

    pub(super) fn run_state(&self, hub: &TestHub, run: Uuid) -> (String, Option<String>) {
        let found = rows(hub, &format!("select state, controller::text from team_runs where id = '{run}'"));
        (found[0][0].clone().unwrap(), found[0][1].clone())
    }

    /// What the box's session got typed for it, read until it holds `text`.
    fn told(&self, hub: &TestHub, text: &str) -> String {
        let mut told = String::new();
        wait_for(&format!("{text:?} in the session"), || {
            if on_node(hub, &self.box_id, |node| node.machine.view(&self.tab).is_ok_and(|v| v.queued > 0)) {
                told += &turn_end_context(&self.dir, &self.tab);
            }
            told.contains(text)
        });
        told
    }
}

/// Runs `f` on the node of box `box_id`.
pub(super) fn on_node<T>(hub: &TestHub, box_id: &str, f: impl FnOnce(&Node) -> T) -> T {
    let node = hub.boxes.state.lock().unwrap().boxes.get(box_id).and_then(|b| b.node.clone()).expect("the box runs a node");
    f(&node)
}

fn ask_from(hub: &TestHub, box_id: &str, question: &str, choices: &[&str]) -> Result<Value, String> {
    let op = TeamOp::AskPeople { situation: Situation::InDoubt, question: question.into(), choices: choices.iter().map(|c| c.to_string()).collect(), step: None };
    on_node(hub, box_id, |node| node.link.as_ref().unwrap().call_hub(HubCall::Team { team: None, op }))
}

pub(super) fn choice(choice: &str, text: &str) -> Value {
    json!({ "kind": { "kind": "choice" }, "choice": choice, "text": text })
}

fn design() -> Value {
    json!({ "kind": "group", "handle": "design" })
}

/// A member's op on team `team`, from their app.
pub(super) fn op(team: Uuid, client: &mut Client, op: Value) -> Result<Value, String> {
    client.hub(json!({ "cmd": "team", "team": team.to_string(), "op": op }))
}

/// The team's calls as a member lists them.
pub(super) fn calls_of(team: Uuid, client: &mut Client) -> Vec<Value> {
    op(team, client, json!({ "op": "calls" })).unwrap().as_array().unwrap().clone()
}

#[test]
fn a_team_box_asks_its_bots_group_and_hears_back_at_once_while_every_member_sees_the_call() {
    let hub = TestHub::start_with_team_policy("teams_calls_ask", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut dina = tb.member(&hub, "dina@example.test", &["design"]);
    let mut omar = tb.member(&hub, "omar@example.test", &["ops"]);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    let team = tb.team;
    for client in [&mut tb.owner.client, &mut dina.client, &mut omar.client] {
        op(team, client, json!({ "op": "runs" })).unwrap();
    }

    let asked_at = Instant::now();
    let asked = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap();
    assert!(asked_at.elapsed() < Duration::from_secs(2), "the reply came at once, not with the answer");
    let call = asked["call"].as_str().unwrap().to_owned();
    assert_eq!(asked, json!({ "call": call, "asked": ["@design"] }));

    for client in [&mut tb.owner.client, &mut dina.client, &mut omar.client] {
        let listed = calls_of(team, client);
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (&listed[0]["id"], &listed[0]["run"], &listed[0]["asked"], &listed[0]["question"], &listed[0]["choices"], &listed[0]["escalated"], &listed[0]["answer"]),
            (&json!(call), &json!(run.to_string()), &json!([design()]), &json!("Red or blue footer?"), &json!(["red", "blue"]), &json!(false), &Value::Null)
        );
        let pushed = client.wait("the call pushed", |f| f["type"] == "team" && f["event"]["kind"] == "call_opened");
        assert_eq!(pushed["event"]["call"]["id"], json!(call));
        let runs = op(team, client, json!({ "op": "runs" })).unwrap();
        assert_eq!((&runs[0]["id"], &runs[0]["state"]), (&json!(run.to_string()), &json!("waiting")));
    }
}

#[test]
fn of_two_members_answering_at_once_exactly_one_wins() {
    let hub = TestHub::start_with_team_policy("teams_calls_race", fast(), patient());
    let tb = TeamBox::new(&hub);
    let dina = tb.member(&hub, "dina@example.test", &["design"]);
    let eva = tb.member(&hub, "eva@example.test", &["design"]);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    let call = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap()["call"].clone();

    let answer = |answer: Value| json!({ "op": "answer_call", "id": call, "answer": answer });
    let (first, second) = hub.runtime.block_on(async {
        tokio::join!(tb.op_of(hub.hub(), dina.id, answer(choice("red", ""))), tb.op_of(hub.hub(), eva.id, answer(choice("blue", ""))))
    });
    let (winner, loser, said) = match (&first, &second) {
        (Ok(won), Err(lost)) => (won, lost, "dina already answered: red"),
        (Err(lost), Ok(won)) => (won, lost, "eva already answered: blue"),
        _ => panic!("exactly one answer wins: {first:?} {second:?}"),
    };
    assert_eq!(loser, said);
    assert_eq!(winner["answer"]["kind"], json!({ "kind": "choice" }));
    let stored = rows(&hub, &format!("select outcome, answer->>'choice' from team_calls where id = '{}'", call.as_str().unwrap()));
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0][0].as_deref(), Some("answered"));
    assert_eq!(tb.run_state(&hub, run), ("working".to_owned(), None));
}

#[test]
fn a_member_the_call_does_not_ask_is_refused_and_the_owner_always_answers() {
    let hub = TestHub::start_with_team_policy("teams_calls_who", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut omar = tb.member(&hub, "omar@example.test", &["ops"]);
    tb.member(&hub, "dina@example.test", &["design"]);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    let call = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap()["call"].clone();

    let answer = |a: Value| json!({ "op": "answer_call", "id": call, "answer": a });
    assert_eq!(op(tb.team, &mut omar.client, answer(choice("red", ""))), Err("this call is not for you".into()));
    assert_eq!(op(tb.team, &mut tb.owner.client, answer(choice("green", ""))), Err("choose one of: red, blue".into()));
    let answered = op(tb.team, &mut tb.owner.client, answer(choice("blue", "with a thin border"))).unwrap();
    assert_eq!((&answered["answer"]["by"], &answered["answer"]["choice"], &answered["answer"]["text"]), (&json!(tb.owner.id.to_string()), &json!("blue"), &json!("with a thin border")));
    assert_eq!(tb.run_state(&hub, run), ("working".to_owned(), None));
}

#[test]
fn an_unanswered_call_goes_to_the_owner_and_then_gives_up_ending_its_box() {
    let quick = TeamPolicy { escalate_after_secs: 1, give_up_after_secs: 3, ..TeamPolicy::default() };
    let hub = TestHub::start_with_team_policy("teams_calls_tick", fast(), quick);
    let mut tb = TeamBox::new(&hub);
    tb.member(&hub, "dina@example.test", &["design"]);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    let asked_at = Instant::now();
    let call = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap()["call"].as_str().unwrap().to_owned();
    let team = tb.team;

    wait_long("the call to escalate", Duration::from_secs(3), || calls_of(team, &mut tb.owner.client)[0]["escalated"] == json!(true));
    assert!(asked_at.elapsed() < Duration::from_secs(3), "escalated before it gave up");
    let listed = calls_of(team, &mut tb.owner.client);
    assert_eq!((&listed[0]["asked"], &listed[0]["answer"]), (&json!([design(), { "kind": "owner" }]), &Value::Null));
    assert_eq!(tb.run_state(&hub, run).0, "waiting");
    let pushed = tb.owner.client.wait("the escalation pushed", |f| f["type"] == "team" && f["event"]["kind"] == "call_opened" && f["event"]["call"]["escalated"] == json!(true));
    assert_eq!(pushed["event"]["call"]["id"], json!(call));

    wait_long("the run to give up", Duration::from_secs(6), || tb.run_state(&hub, run).0 == "gave_up");
    assert!(asked_at.elapsed() >= Duration::from_secs(3));
    let stored = rows(&hub, &format!("select outcome, outcome from team_runs where id = '{run}' union all select outcome, answer::text from team_calls where id = '{call}'"));
    assert_eq!(stored, [[Some(format!("nobody answered call {call}")), Some(format!("nobody answered call {call}"))], [Some("gave-up".to_owned()), None]]);
    wait_for("the box to be deleted", || hub.boxes.live().is_empty());
    assert_eq!(
        op(team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": choice("red", "") })),
        Err("nobody answered this call in time: its run gave up".into())
    );
}

#[test]
fn an_answer_racing_the_give_up_leaves_exactly_one_outcome() {
    let hub = TestHub::start_with_team_policy("teams_calls_give_up_race", fast(), patient());
    let tb = TeamBox::new(&hub);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    let db = &hub.hub().db;
    for _ in 0..5 {
        let open = pinned_core::teams::Call {
            id: Uuid::new_v4().to_string(),
            run: run.to_string(),
            workflow: tb.workflow.clone(),
            situation: Situation::InDoubt,
            asked: vec![pinned_core::teams::Who::Owner],
            question: "Ship it?".into(),
            choices: vec![],
            created_at: 0,
            escalated: false,
            answer: None,
        };
        let call = Uuid::parse_str(&hub.runtime.block_on(teams::db::insert_call(db, tb.team, &open, None)).unwrap().id).unwrap();
        let answer = pinned_core::teams::Answer { by: tb.owner.id.to_string(), choice: None, text: "ship".into(), kind: pinned_core::teams::AnswerKind::Choice, at: 0 };
        let (answered, gave_up) = hub.runtime.block_on(async { tokio::join!(teams::db::answer_call(db, tb.team, call, &answer, "answered"), teams::db::give_up_due(db, 0)) });
        let (answered, gave_up) = (answered.unwrap().is_some(), gave_up.unwrap().iter().any(|row| row.call.id == call.to_string()));
        assert!(answered != gave_up, "exactly one wins: answered {answered}, gave up {gave_up}");
        let stored = rows(&hub, &format!("select outcome from team_calls where id = '{call}'"));
        assert_eq!(stored, [[Some(if answered { "answered" } else { "gave-up" }.to_owned())]]);
    }
}

#[test]
fn only_a_team_box_with_a_live_run_and_no_open_call_asks_people() {
    let hub = TestHub::start_with_team_policy("teams_calls_refused", fast(), patient());
    let tb = TeamBox::new(&hub);
    tb.bot(&hub, design(), None);
    assert_eq!(tb.ask(&hub, "Red or blue footer?", &[]), Err("no run of the team works in this box".into()));

    let pia = BoxAccount::named(&hub, "pia@example.test");
    assert_eq!(hub.runtime.block_on(hub.hub().open(pia.id, pia.box_run(issue("pia-1")))), Ok(()));
    let row = hub.runtime.block_on(hub.hub().db.boxes(pia.id)).unwrap().into_iter().next().unwrap().unwrap();
    wait_for("pia's box node", || hub.boxes.state.lock().unwrap().boxes.get(&row.upstash.as_ref().unwrap().0).is_some_and(|b| b.node.is_some()));
    assert_eq!(ask_from(&hub, &row.upstash.unwrap().0, "Red or blue footer?", &[]), Err("only a session in one of a team's boxes asks people".into()));

    tb.run(&hub);
    tb.ask(&hub, "Red or blue footer?", &[]).unwrap();
    assert_eq!(tb.ask(&hub, "And the header?", &[]), Err("this run already waits on a call".into()));
}

#[test]
fn passing_a_call_on_closes_it_and_asks_the_other_group_on_the_same_run() {
    let hub = TestHub::start_with_team_policy("teams_calls_pass", fast(), patient());
    let tb = TeamBox::new(&hub);
    let mut dina = tb.member(&hub, "dina@example.test", &["design"]);
    let mut omar = tb.member(&hub, "omar@example.test", &["ops"]);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    let first = tb.ask(&hub, "Deploy now?", &[]).unwrap()["call"].clone();
    let team = tb.team;

    let pass = |handle: &str| json!({ "op": "answer_call", "id": first, "answer": { "kind": { "kind": "pass_to", "handle": handle } } });
    assert_eq!(op(team, &mut dina.client, pass("@nobody")), Err("the team has no group @nobody".into()));
    let passed = op(team, &mut dina.client, pass("@ops")).unwrap();
    assert_eq!((&passed["answer"]["kind"], &passed["answer"]["by"]), (&json!({ "kind": "pass_to", "handle": "ops" }), &json!(dina.id.to_string())));
    let listed = calls_of(team, &mut omar.client);
    assert_eq!(listed.len(), 2);
    assert_eq!((&listed[0]["run"], &listed[0]["asked"], &listed[0]["question"], &listed[0]["answer"]), (&json!(run.to_string()), &json!([{ "kind": "group", "handle": "ops" }]), &json!("Deploy now?"), &Value::Null));
    assert_eq!(listed[1]["id"], first);
    assert_eq!(tb.run_state(&hub, run).0, "waiting");
    let second = listed[0]["id"].clone();
    assert_eq!(op(team, &mut dina.client, json!({ "op": "answer_call", "id": second, "answer": choice("", "yes") })), Err("this call is not for you".into()));
    op(team, &mut omar.client, json!({ "op": "answer_call", "id": second, "answer": choice("", "yes, now") })).unwrap();
    assert_eq!(tb.run_state(&hub, run).0, "working");
    tb.told(&hub, &format!("[Answer from omar on call {}: yes, now]", second.as_str().unwrap()));
}

#[test]
fn one_member_takes_over_gives_back_and_a_stop_ends_the_box() {
    let hub = TestHub::start_with_team_policy("teams_calls_take_over", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut bea = tb.member(&hub, "bea@example.test", &[]);
    let mut omar = tb.member(&hub, "omar@example.test", &["ops"]);
    tb.bot(&hub, design(), Some(bea.id));
    let run = tb.run(&hub);
    let team = tb.team;
    let on_run = |name: &str| json!({ "op": name, "run": run.to_string() });

    assert_eq!(op(team, &mut omar.client, on_run("take_over")), Err("only the people its call asks, its bot's owner or the team's owner do this".into()));
    let taken = op(team, &mut bea.client, on_run("take_over")).unwrap();
    assert_eq!((&taken["state"], &taken["controller"]), (&json!("taken_over"), &json!(bea.id.to_string())));
    assert_eq!(op(team, &mut tb.owner.client, on_run("take_over")), Err("bea already took over".into()));
    assert_eq!(op(team, &mut omar.client, on_run("give_back")), Err("only bea, who took it over, or the team's owner gives it back".into()));
    let back = op(team, &mut bea.client, on_run("give_back")).unwrap();
    assert_eq!((&back["state"], &back["controller"]), (&json!("working"), &Value::Null));
    tb.told(&hub, "[bea gave the run back: continue]");

    let stopped = op(team, &mut tb.owner.client, on_run("stop_run")).unwrap();
    assert_eq!((&stopped["state"], &stopped["outcome"]), (&json!("stopped"), &json!("stopped by olga")));
    wait_for("the box to be deleted", || hub.boxes.live().is_empty());
    assert_eq!(op(team, &mut tb.owner.client, on_run("stop_run")), Err("this run has ended".into()));
}

#[test]
fn a_box_waiting_on_people_is_not_ended_for_idleness_but_still_at_its_max_life() {
    let idle = Policy { idle: Duration::from_secs(1), ..fast() };
    let hub = TestHub::start_with_team_policy("teams_calls_idle", idle, patient());
    let mut tb = TeamBox::new(&hub);
    tb.bot(&hub, design(), None);
    tb.run(&hub);
    let call = tb.ask(&hub, "Red or blue footer?", &[]).unwrap()["call"].clone();
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(hub.boxes.live().len(), 1, "a box whose run waits on people outlasts its idle time");
    op(tb.team, &mut tb.owner.client, json!({ "op": "answer_call", "id": call, "answer": choice("", "blue") })).unwrap();
    wait_for("the idle box to be deleted once answered", || hub.boxes.live().is_empty());
    drop(hub);

    let short = Policy { idle: Duration::from_secs(1), max_life: Duration::from_secs(4), ..fast() };
    let hub = TestHub::start_with_team_policy("teams_calls_max_life", short, patient());
    let tb = TeamBox::new(&hub);
    tb.bot(&hub, design(), None);
    tb.run(&hub);
    tb.ask(&hub, "Red or blue footer?", &[]).unwrap();
    let started = hub.runtime.block_on(hub.hub().db.boxes(tb.team)).unwrap().into_iter().next().unwrap().unwrap().age;
    wait_long("the box to end at its max life", Duration::from_secs(12), || hub.boxes.live().is_empty());
    assert!(started < Duration::from_secs(4), "it was asked before its max life");
}

#[test]
fn the_answer_is_typed_into_the_session_that_asked() {
    let hub = TestHub::start_with_team_policy("teams_calls_typed", fast(), patient());
    let mut tb = TeamBox::new(&hub);
    let mut dina = tb.member(&hub, "dina@example.test", &["design"]);
    tb.bot(&hub, design(), None);
    let run = tb.run(&hub);
    let call = tb.ask(&hub, "Red or blue footer?", &["red", "blue"]).unwrap()["call"].as_str().unwrap().to_owned();
    op(tb.team, &mut dina.client, json!({ "op": "answer_call", "id": call, "answer": choice("red", "and keep the logo") })).unwrap();
    let told = tb.told(&hub, "[Answer from");
    assert_eq!(told, format!("[Answer from dina on call {call}: red and keep the logo]"));
    assert_eq!(tb.run_state(&hub, run), ("working".to_owned(), None));
    let resolved = tb.owner.client.wait("the answer pushed", |f| f["type"] == "team" && f["event"]["kind"] == "call_resolved");
    assert_eq!(resolved["event"]["call"]["answer"]["choice"], json!("red"));
}
