//! Groups: people a bot asks by one handle (`@design`).

use super::{db, Scope};
use crate::hub::Hub;
use pinned_core::teams::{Group, Role};
use std::sync::Arc;

/// The longest handle, in characters, without its `@`.
pub const HANDLE_CHARS: usize = 32;
/// The longest `about`, in characters.
pub const ABOUT_CHARS: usize = 280;
/// The longest Slack group, in characters.
pub const SLACK_CHARS: usize = 80;

/// A handle without its `@`, or why it is not one: lowercase letters, digits and `-`, 2 to 32
/// characters.
pub fn handle_name(raw: &str) -> Result<&str, String> {
    let handle = raw.strip_prefix('@').unwrap_or(raw);
    let len = handle.chars().count();
    let plain = handle.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if (2..=HANDLE_CHARS).contains(&len) && plain {
        Ok(handle)
    } else {
        Err(format!("a group's handle is 2 to {HANDLE_CHARS} lowercase letters, digits or dashes, like @design"))
    }
}

/// The groups of the team with their members. Any member reads them.
pub async fn groups(hub: &Arc<Hub>, scope: &Scope) -> Result<Vec<Group>, String> {
    db::groups(&hub.db, scope.team.id).await
}

/// Makes or edits group `group.handle`; any member may. Its `about`, `fallback` and `slack_group`
/// are set as given. Its members are set by the owner only: the owner's save replaces them, and
/// a member's save keeps the stored members of a group that exists, and makes a new group with
/// only the member in it. `group.members` of a member's save is ignored.
pub async fn save_group(hub: &Arc<Hub>, scope: &Scope, mut group: Group) -> Result<(), String> {
    group.handle = handle_name(&group.handle)?.to_owned();
    if group.about.chars().count() > ABOUT_CHARS || group.about.contains('\0') {
        return Err(format!("a group's description is at most {ABOUT_CHARS} characters"));
    }
    if group.slack_group.chars().count() > SLACK_CHARS || group.slack_group.contains('\0') {
        return Err(format!("a group's Slack group is at most {SLACK_CHARS} characters"));
    }
    let fallback = group.fallback.take().filter(|f| !f.is_empty());
    if let Some(fallback) = &fallback {
        super::members_only(hub, scope.team.id, &[fallback.as_str()]).await?;
    }
    group.fallback = fallback;
    // Checked for a member's save too, though only the owner's decides the members: no role probes.
    let ids = super::members_only(hub, scope.team.id, &group.members.iter().map(String::as_str).collect::<Vec<_>>()).await?;
    let caller = [scope.member];
    let members = match scope.role {
        Role::Owner => db::Members::Replace(&ids),
        Role::Member => db::Members::Keep { new: &caller },
    };
    db::save_group(&hub.db, scope.team.id, &group, members).await
}

/// The member joins group `handle`; joining a group it is in already changes nothing.
pub async fn join_group(hub: &Arc<Hub>, scope: &Scope, handle: &str) -> Result<(), String> {
    let handle = handle_name(handle)?;
    if db::join_group(&hub.db, scope.team.id, handle, scope.member).await? {
        Ok(())
    } else {
        Err(format!("there is no group @{handle}"))
    }
}

/// The member leaves group `handle`.
pub async fn leave_group(hub: &Arc<Hub>, scope: &Scope, handle: &str) -> Result<(), String> {
    let handle = handle_name(handle)?;
    if db::leave_group(&hub.db, scope.team.id, handle, scope.member).await? {
        Ok(())
    } else {
        Err(format!("you are not in @{handle}"))
    }
}
