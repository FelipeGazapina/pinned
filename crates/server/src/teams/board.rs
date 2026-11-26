//! The team's work board on the hub (`pinned_core::work_board`): its setup and cards, the acts of
//! members, the reports of the agents' runs, and the dispatch that gives each free seated agent
//! its next card by starting a run of the seat's bot. Every change of a card is pushed to the
//! members as `TeamEvent::CardChanged`, then the board is dispatched again.

use super::bots::Start;
use super::queue::{self, Asked};
use super::{db, push, Scope};
use crate::hub::{Account, Hub};
use pinned_core::teams::TeamEvent;
use pinned_core::work_board::{next_picks, with_profile, Agent, BoardSetup, BoardView, Card, CardAct, Report};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use uuid::Uuid;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn read<T: serde::de::DeserializeOwned>(value: serde_json::Value, what: &str) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("the team's {what} does not read: {e}"))
}

fn parse_id(id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id).map_err(|_| "the board has no such card".to_owned())
}

/// The team's setup: the default until the owner saves one.
pub async fn setup(hub: &Hub, team: Uuid) -> Result<BoardSetup, String> {
    db::board_setup(&hub.db, team).await?.map_or_else(|| Ok(BoardSetup::default()), |v| read(v, "board setup"))
}

async fn cards(hub: &Hub, team: Uuid) -> Result<Vec<Card>, String> {
    db::cards(&hub.db, team).await?.into_iter().map(|v| read(v, "card")).collect()
}

/// Any member.
pub async fn board(hub: &Arc<Hub>, s: &Scope) -> Result<BoardView, String> {
    Ok(BoardView { setup: setup(hub, s.team.id).await?, cards: cards(hub, s.team.id).await? })
}

/// Any member: a card in Backlog, in one of the team's projects when it names one.
pub async fn add_card(hub: &Arc<Hub>, s: &Scope, title: &str, brief: &str, project: &str) -> Result<Card, String> {
    let project = project.trim();
    if !project.is_empty() && !db::projects(&hub.db, s.team.id).await?.iter().any(|p| p.key == project) {
        return Err(format!("the team has no project {project}"));
    }
    let id = Uuid::new_v4();
    let max_rounds = setup(hub, s.team.id).await?.max_rounds;
    let card = Card::new(id.to_string(), title, brief, project, &s.member.to_string(), max_rounds, now())?;
    db::insert_card(&hub.db, s.team.id, id, &super::json(&card)?).await?;
    changed(hub, &s.team, &card).await;
    Ok(card)
}

/// Any member: queue, resume, send back, pass or mark merged.
pub async fn card_act(hub: &Arc<Hub>, s: &Scope, card: &str, act: CardAct) -> Result<Card, String> {
    let setup = setup(hub, s.team.id).await?;
    let member = s.member.to_string();
    change(hub, &s.team, parse_id(card)?, |c| c.act(act.clone(), &member, &setup, now())).await
}

/// Owner only. Each seat's bot must be one of the team's bots.
pub async fn set_setup(hub: &Arc<Hub>, s: &Scope, setup: BoardSetup) -> Result<BoardSetup, String> {
    s.owner_only()?;
    setup.check()?;
    let bots = super::bots::bots(hub, s).await?;
    for agent in Agent::ALL {
        if let Some(bot) = &setup.seat(agent).bot {
            if !bots.iter().any(|b| &b.workflow == bot) {
                return Err(format!("the {}'s seat names no bot of the team", agent.name()));
            }
        }
    }
    db::save_board_setup(&hub.db, s.team.id, &super::json(&setup)?).await?;
    dispatch(hub, &s.team);
    Ok(setup)
}

/// From a team's box: the agent whose run works in `machine` reports on its card.
pub async fn card_report(hub: &Arc<Hub>, team: &Arc<Account>, machine: &str, report: Report) -> Result<Card, String> {
    const NOT_A_CARD_RUN: &str = "no run of the team's board works in this box";
    let machine_id = Uuid::parse_str(machine).map_err(|_| NOT_A_CARD_RUN)?;
    if !db::is_team(&hub.db, team.id).await? || hub.db.box_of(team.id, machine_id).await?.is_none() {
        return Err(NOT_A_CARD_RUN.into());
    }
    let run = db::run_on_machine(&hub.db, team.id, machine).await?.ok_or(NOT_A_CARD_RUN)?;
    let card = cards(hub, team.id).await?.into_iter().find(|c| c.run.as_deref() == Some(run.id.as_str())).ok_or(NOT_A_CARD_RUN)?;
    let setup = setup(hub, team.id).await?;
    let scored = matches!(report, Report::Scored { .. });
    let card = change(hub, team, parse_id(&card.id)?, |c| c.report(&run.id, report.clone(), &setup, now())).await?;
    if scored {
        let (hub, team, card) = (hub.clone(), team.clone(), card.clone());
        tokio::spawn(async move { approver_check(&hub, &team, &card).await });
    }
    Ok(card)
}

/// Run `run` of the team ended (`calls::end_run`): a card it still worked waits on a person.
pub async fn run_ended(hub: &Hub, team: &Arc<Account>, run: &str) {
    let Some(hub) = hub.arc() else { return };
    let ended = async {
        for card in cards(&hub, team.id).await? {
            if card.run.as_deref() == Some(run) {
                change(&hub, team, parse_id(&card.id)?, |c| if c.run_ended(run, now()) { Ok(()) } else { Err("moved on".into()) }).await?;
            }
        }
        Ok::<_, String>(())
    };
    if let Err(e) = ended.await {
        eprintln!("pinned-server: team {}: the card of ended run {run} was not put back: {e}", team.id);
    }
    dispatch(&hub, team);
}

/// Applies `f` to card `id` and writes it while nobody changed it meanwhile, a few times over.
async fn change(hub: &Arc<Hub>, team: &Arc<Account>, id: Uuid, f: impl Fn(&mut Card) -> Result<(), String>) -> Result<Card, String> {
    for _ in 0..5 {
        let old = db::card(&hub.db, team.id, id).await?.ok_or("the board has no such card")?;
        let mut card: Card = read(old.clone(), "card")?;
        f(&mut card)?;
        if db::save_card_if(&hub.db, team.id, id, &old, &super::json(&card)?).await? {
            changed(hub, team, &card).await;
            return Ok(card);
        }
    }
    Err(crate::db::BUSY.into())
}

async fn changed(hub: &Arc<Hub>, team: &Arc<Account>, card: &Card) {
    if let Err(e) = push(hub, team.id, TeamEvent::CardChanged { card: card.clone() }).await {
        eprintln!("pinned-server: team {}: card {} changed but was not pushed: {e}", team.id, card.id);
    }
    dispatch(hub, team);
}

/// One dispatch at a time per team in this hub.
fn turn(team: Uuid) -> Arc<tokio::sync::Mutex<()>> {
    static TURNS: LazyLock<Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Mutex::default);
    TURNS.lock().unwrap().entry(team).or_default().clone()
}

/// Gives each free seated agent its next card, in a task of its own.
pub fn dispatch(hub: &Arc<Hub>, team: &Arc<Account>) {
    let (hub, team) = (hub.clone(), team.clone());
    tokio::spawn(async move {
        let _turn = turn(team.id).lock_owned().await;
        if let Err(e) = dispatch_now(&hub, &team).await {
            eprintln!("pinned-server: team {}: the board did not dispatch: {e}", team.id);
        }
    });
}

async fn dispatch_now(hub: &Arc<Hub>, team: &Arc<Account>) -> Result<(), String> {
    let setup = setup(hub, team.id).await?;
    let cards = cards(hub, team.id).await?;
    for (id, agent) in next_picks(&cards, &setup) {
        let Some(bot) = setup.seat(agent).bot.clone() else { continue };
        // A bot still finishing a run takes the card when that run ends (`run_ended`).
        if !db::live_runs_of(&hub.db, team.id, &bot).await?.is_empty() {
            continue;
        }
        if db::live_count(&hub.db, team.id).await? >= db::max_live_runs(&hub.db, team.id).await? {
            break;
        }
        let Some(card) = cards.iter().find(|c| c.id == id) else { continue };
        let card_id = parse_id(&card.id)?;
        let started = async {
            let mut work = hub.connectors(team).await?.run_now(&bot, "the team's work board")?;
            if let Some(profile) = setup.profile(agent) {
                work.args = with_profile(work.args, profile);
            }
            if let Some(last) = work.args.last_mut() {
                *last = format!("{last}\n\n{}", card.brief(agent, &setup));
            }
            queue::ask(hub, team, &bot, Start::Trigger { kind: "card".into(), work }).await
        };
        match started.await {
            Ok(Asked::Started(run, _)) => {
                change(hub, team, card_id, |c| c.pick(&run.id, now()).map(drop)).await?;
            }
            Ok(Asked::Waiting { .. }) => eprintln!("pinned-server: team {}: card {id} waits in bot {bot}'s line", team.id),
            Err(e) => {
                let why = format!("The {}'s bot could not start: {e}", agent.name());
                change(hub, team, card_id, |c| {
                    c.person = Some(why.clone());
                    Ok(())
                })
                .await?;
            }
        }
    }
    Ok(())
}

/// Sets the `pinned / approver` status on the head of the card's PR from the Approver's verdict,
/// with the team's own GitHub key. The PR's author cannot approve it, so the check is the verdict.
async fn approver_check(hub: &Arc<Hub>, team: &Arc<Account>, card: &Card) {
    let (Some(pr), Some(verdict)) = (&card.pr, &card.verdict) else { return };
    let posted = async {
        let token = hub.connectors(team).await?.github_token().ok_or("the team has no GitHub key")?;
        let repo = pr.url.strip_prefix("https://github.com/").and_then(|path| path.split("/pull/").next()).filter(|r| r.split('/').count() == 2).ok_or("the PR's link is not a GitHub pull request")?;
        let github = pinned_core::github::Client::new(&token);
        let pull = github.get(&format!("/repos/{repo}/pulls/{}", pr.number)).await?;
        let sha = pull["head"]["sha"].as_str().ok_or("the PR has no head commit")?;
        let state = if verdict.score >= verdict.bar { "success" } else { "failure" };
        let description: String = format!("{}/{}: {}", verdict.score, verdict.bar, verdict.reason).chars().take(140).collect();
        github.post(&format!("/repos/{repo}/statuses/{sha}"), &serde_json::json!({ "state": state, "context": "pinned / approver", "description": description })).await?;
        Ok::<_, String>(())
    };
    if let Err(e) = posted.await {
        eprintln!("pinned-server: team {}: the pinned / approver check of card {} was not set: {e}", team.id, card.id);
    }
}
