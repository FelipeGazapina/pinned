//! Groups: people a bot asks by one handle (`@design`).

use super::Scope;
use crate::hub::Hub;
use pinned_core::teams::Group;
use std::sync::Arc;

/// The team's groups with their members. Any member reads them.
pub async fn groups(_hub: &Arc<Hub>, _scope: &Scope) -> Result<Vec<Group>, String> {
    Err("not built yet".into())
}

/// Owner only. Makes or replaces group `group.handle`: a handle of lowercase letters, digits and
/// `-`, its members and fallback members of the team.
pub async fn save_group(_hub: &Arc<Hub>, _scope: &Scope, _group: Group) -> Result<(), String> {
    Err("not built yet".into())
}

/// The member joins group `handle`.
pub async fn join_group(_hub: &Arc<Hub>, _scope: &Scope, _handle: &str) -> Result<(), String> {
    Err("not built yet".into())
}

/// The member leaves group `handle`.
pub async fn leave_group(_hub: &Arc<Hub>, _scope: &Scope, _handle: &str) -> Result<(), String> {
    Err("not built yet".into())
}
