//! A team's work board end to end: a card a member queues is picked by the Builder's bot in a box,
//! the box reports the PR, the Reviewer's bot sends it back once, the Builder pushes to the same
//! PR, the Reviewer passes it, and a person merges it.

use super::*;
use pinned_core::teams::{TeamOp, TeamPolicy};
use pinned_core::wire::HubCall;
use pinned_core::work_board::{BoardSetup, Card, Report};
use pinned_server::hub::Account;
use pinned_server::teams;
use uuid::Uuid;

struct Board {
    hub: TestHub,
    team: Uuid,
    ana: Arc<Account>,
    bob: Arc<Account>,
    folder: PathBuf,
}

impl Board {
    fn new(name: &str) -> Board {
        let patient = TeamPolicy { escalate_after_secs: 3600, give_up_after_secs: 7200, ..TeamPolicy::default() };
        let hub = TestHub::start_with_team_policy(name, fast(), patient);
        let sign_up = |email: &str| {
            let (_, view) = account::signup(&hub.url, email, "correct horse").unwrap();
            hub.runtime.block_on(hub.hub().account(Uuid::parse_str(&view.id).unwrap())).unwrap()
        };
        let (ana, bob) = (sign_up("ana@example.test"), sign_up("bob@example.test"));
        let made = hub.runtime.block_on(teams::call(hub.hub(), &ana, None, teams::Op::Team(TeamOp::Create { name: "Acme".into() }))).unwrap();
        let team = Uuid::parse_str(made["id"].as_str().unwrap()).unwrap();
        admin_on(&hub.database, &format!("insert into team_members (team, member, role) values ('{team}', '{}', 'member')", bob.id));
        // One run at a time, so the one live box is the agent's whose turn it is.
        admin_on(&hub.database, &format!("update teams set max_live_runs = 1 where id = '{team}'"));
        let board = Board { hub, team, ana, bob, folder: temp(&format!("teams-board-{name}")) };
        board.call(&board.ana, TeamOp::Hub { call: json!({ "cmd": "connect_upstash", "key": BOX_KEY, "claude_token": CLAUDE_TOKEN }) }).unwrap();
        board
    }

    fn call(&self, who: &Arc<Account>, op: TeamOp) -> Result<Value, String> {
        self.hub.runtime.block_on(teams::call(self.hub.hub(), who, Some(self.team), teams::Op::read(op)?))
    }

    fn on_bot(&self, name: &str) -> String {
        let id = save_team_workflow(&self.hub, &self.bob, self.team, json!({
            "name": name, "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": self.folder, "model": "", "effort": "high", "permissionMode": "acceptEdits", "worktree": false,
            "instructions": format!("You are the {name}."),
        }))
        .unwrap();
        self.call(&self.bob, serde_json::from_value(json!({ "op": "save_bot", "bot": { "workflow": id } })).unwrap()).unwrap();
        self.call(&self.ana, TeamOp::TurnBotOn { workflow: id.clone(), on: true }).unwrap();
        id
    }

    fn card(&self, id: &str) -> Card {
        let view = self.call(&self.bob, TeamOp::Board).unwrap();
        serde_json::from_value(view["cards"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap().clone()).unwrap()
    }

    /// What the session in the team's one live box reports.
    fn report(&self, report: Report) -> Result<Value, String> {
        let node = self.hub.boxes.state.lock().unwrap().boxes.values().find_map(|b| b.node.clone()).expect("a box runs a node");
        node.link.as_ref().unwrap().call_hub(HubCall::Team { team: None, op: TeamOp::CardReport { report } })
    }

    /// Ends the run that reported, as its session ending would, and waits for its box to go.
    fn end_run(&self, run: &str) {
        self.call(&self.ana, serde_json::from_value(json!({ "op": "stop_run", "run": run })).unwrap()).unwrap();
        wait_for("the box to end", || self.hub.boxes.live().is_empty());
    }
}

fn built(pr: u64) -> Report {
    Report::Built { pr, url: format!("https://github.com/acme/site/pull/{pr}"), branch: "card/x".into() }
}

#[test]
fn a_card_goes_from_backlog_to_done_through_the_builder_and_reviewer_on_one_pr() {
    let b = Board::new("board_flow");
    let (builder, reviewer) = (b.on_bot("Builder"), b.on_bot("Reviewer"));
    let mut setup = BoardSetup::default();
    setup.builder.bot = Some(builder);
    setup.reviewer.bot = Some(reviewer);
    assert_eq!(b.call(&b.bob, TeamOp::SetBoardSetup { setup: setup.clone() }), Err("only the team's owner does this".into()));
    b.call(&b.ana, TeamOp::SetBoardSetup { setup }).unwrap();

    let card: Card = serde_json::from_value(b.call(&b.bob, TeamOp::AddCard { title: "Fix Safari export".into(), brief: "The button does nothing.".into(), project: String::new() }).unwrap()).unwrap();
    b.call(&b.bob, serde_json::from_value(json!({ "op": "card_act", "card": card.id, "act": { "act": "queue" } })).unwrap()).unwrap();

    wait_for("the Builder to pick the card", || b.card(&card.id).column == pinned_core::work_board::Column::InDev);
    wait_for("the Builder's box", || b.hub.boxes.live().len() == 1);
    let run = b.card(&card.id).run.unwrap();
    assert_eq!(b.report(Report::Reviewed { passed: true, report: "ok".into(), request: String::new() }), Err("the card is the Builder's now".into()));
    b.report(built(12)).unwrap();
    b.end_run(&run);

    wait_for("the Reviewer to pick it", || b.card(&card.id).run.is_some() && b.card(&card.id).turn == Some(pinned_core::work_board::Agent::Reviewer));
    wait_for("the Reviewer's box", || b.hub.boxes.live().len() == 1);
    let run = b.card(&card.id).run.unwrap();
    b.report(Report::Reviewed { passed: false, report: "1 test failed".into(), request: "the empty list panics".into() }).unwrap();
    b.end_run(&run);

    wait_for("the Builder's second round", || b.card(&card.id).round == 2 && b.card(&card.id).run.is_some());
    wait_for("its box", || b.hub.boxes.live().len() == 1);
    let run = b.card(&card.id).run.unwrap();
    assert!(b.report(built(13)).unwrap_err().contains("never a second PR"));
    b.report(built(12)).unwrap();
    b.end_run(&run);

    wait_for("the Reviewer again", || b.card(&card.id).run.is_some() && b.card(&card.id).turn == Some(pinned_core::work_board::Agent::Reviewer));
    wait_for("its box", || b.hub.boxes.live().len() == 1);
    let run = b.card(&card.id).run.unwrap();
    b.report(Report::Reviewed { passed: true, report: "212 passed".into(), request: String::new() }).unwrap();
    b.end_run(&run);

    let ready = b.card(&card.id);
    assert_eq!((ready.column, ready.pr.unwrap().number, ready.requests.len()), (pinned_core::work_board::Column::ReadyToDeploy, 12, 1));
    b.call(&b.bob, serde_json::from_value(json!({ "op": "card_act", "card": card.id, "act": { "act": "merged" } })).unwrap()).unwrap();
    assert_eq!(b.card(&card.id).column, pinned_core::work_board::Column::Done);
}
