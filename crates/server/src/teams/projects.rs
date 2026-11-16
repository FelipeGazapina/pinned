//! The projects a team's bots work in: members' own shared projects, attached by key.

use super::{db, Scope};
use crate::hub::Hub;
use pinned_core::teams::{Role, TeamProject};
use std::sync::Arc;

/// Attaches the member's own shared project `key` (one of `db.shared_projects` of `Scope::member`,
/// read by the member's id only) to the team, with its name and repo, as attached by the member.
/// Only the key, name and repo cross: no file, session, memory or token. Attaching a key the team
/// already has changes nothing, whoever attached it.
pub async fn attach_project(hub: &Arc<Hub>, scope: &Scope, key: &str) -> Result<(), String> {
    let own = hub.db.shared_projects(scope.member).await?;
    let project = own.into_iter().find(|p| p.key == key).ok_or("you can only attach a project you have")?;
    let attached = TeamProject { key: project.key, name: project.name, repo: project.repo, member: scope.member.to_string() };
    db::attach_project(&hub.db, scope.team.id, &attached).await
}

/// Detaches project `key`: the member who attached it or the owner.
pub async fn detach_project(hub: &Arc<Hub>, scope: &Scope, key: &str) -> Result<(), String> {
    let gone = match scope.role {
        Role::Owner => db::detach_project(&hub.db, scope.team.id, key).await?,
        Role::Member => db::detach_project_attached_by(&hub.db, scope.team.id, key, scope.member).await?,
    };
    if gone {
        return Ok(());
    }
    let exists = db::projects(&hub.db, scope.team.id).await?.iter().any(|p| p.key == key);
    Err(if exists { "only the team's owner or the member who attached a project detaches it".into() } else { format!("the team has no project {key}") })
}

/// The team's projects. Any member reads them.
pub async fn projects(hub: &Arc<Hub>, scope: &Scope) -> Result<Vec<TeamProject>, String> {
    db::projects(&hub.db, scope.team.id).await
}
