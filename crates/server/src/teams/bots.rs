//! Bots: the team account's workflows, with who they ask and what a run may spend.

use super::Scope;
use crate::hub::Hub;
use pinned_core::teams::{Bot, Run};
use std::sync::Arc;

/// Every workflow of the team's account as a bot: its `team_bots` row, or `CallRules::default()`
/// and the hub's `TeamPolicy` limits when it has none, with `name` from the workflow.
pub async fn bots(_hub: &Arc<Hub>, _scope: &Scope) -> Result<Vec<Bot>, String> {
    Err("not built yet".into())
}

/// Owner, or the bot's own `owner`. Saves who the bot asks and its limits for workflow
/// `bot.workflow` of the team's account; `on` and `on_by` are left as they are (`turn_bot_on`).
/// Every `Who` must name a member or a group of the team.
pub async fn save_bot(_hub: &Arc<Hub>, _scope: &Scope, _bot: Bot) -> Result<(), String> {
    Err("not built yet".into())
}

/// Any member. Turns the bot on or off, keeping who did it (`on_by`), and enables or disables
/// the team account's workflow to match.
pub async fn turn_bot_on(_hub: &Arc<Hub>, _scope: &Scope, _workflow: &str, _on: bool) -> Result<(), String> {
    Err("not built yet".into())
}

/// Any member. Runs the bot now on the team's account (boxes, keys and Claude token of the team),
/// held to the bot's `BotLimits`, records the run (`db::insert_run`, started by the member) and
/// pushes `TeamEvent::RunChanged`.
pub async fn run_now(_hub: &Arc<Hub>, _scope: &Scope, _workflow: &str) -> Result<Run, String> {
    Err("not built yet".into())
}
