//! The projects a team's bots work in: members' own shared projects, attached by key.

use super::Scope;
use crate::hub::Hub;
use pinned_core::teams::TeamProject;
use std::sync::Arc;

/// Attaches the member's own shared project `key` (one of `db.shared_projects` of `Scope::member`,
/// read by the member's id only) to the team, with its name and repo, as attached by the member.
/// An error when the member shares no such project or the team has one with that key.
pub async fn attach_project(_hub: &Arc<Hub>, _scope: &Scope, _key: &str) -> Result<(), String> {
    Err("not built yet".into())
}

/// Detaches project `key`: the member who attached it or the owner.
pub async fn detach_project(_hub: &Arc<Hub>, _scope: &Scope, _key: &str) -> Result<(), String> {
    Err("not built yet".into())
}

/// The team's projects. Any member reads them.
pub async fn projects(_hub: &Arc<Hub>, _scope: &Scope) -> Result<Vec<TeamProject>, String> {
    Err("not built yet".into())
}
