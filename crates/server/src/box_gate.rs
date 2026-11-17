//! What the hub takes from a box machine of a person's account. A box is a machine of the account
//! that opened it, and the account's other machines load and run what reaches its synced and
//! shared files, so a box may only pull the account's files: the hub refuses every call of a box
//! that would change what the account's other machines or clients read.

use pinned_core::wire::HubCall;

/// The refusal a box's write gets. It starts with `sync::LIMIT_REFUSAL`, so a node that reads a
/// refused push as the account's size limit backs off its push and still pulls in the same cycle.
pub const BOX_REFUSAL: &str = "the account's synced projects would pass 0 MB from a box: a box only pulls the account's files";

/// Whether a box machine may send `call`: a read, or a callback of the box's own run. Every call
/// is classed here by name, so a call added later is classed by whoever adds it.
pub fn box_may_send(call: &HubCall) -> Result<(), String> {
    match call {
        // Reads: what the account has, and what its machines report.
        HubCall::GetSettings
        | HubCall::LinearSuggestions
        | HubCall::SlackSuggestions { .. }
        | HubCall::GithubRepos
        | HubCall::VercelModels
        | HubCall::SharedProjects
        | HubCall::SyncList { .. }
        | HubCall::SyncPull { .. }
        | HubCall::ListSchedules
        | HubCall::ScheduleRuns { .. }
        | HubCall::BoxStatus { .. }
        | HubCall::Machines
        | HubCall::MachineStates
        | HubCall::Place { .. }
        | HubCall::Invites
        | HubCall::ExternalAgents
        | HubCall::Overview
        | HubCall::ReadCaptain { .. }
        | HubCall::GetTastes
        | HubCall::GetBoxSize { .. }
        | HubCall::RepoLook { .. }
        | HubCall::ReadSession { .. }
        | HubCall::DelegationStats { .. }
        | HubCall::UnclaimedEnds
        | HubCall::BoxUsage
        | HubCall::StuckBoxes
        | HubCall::SessionMarks
        | HubCall::Confirmations
        | HubCall::AgentAudit { .. }
        | HubCall::ListMcpServers
        | HubCall::ListIdeas { .. }
        | HubCall::LoadMcpServers
        | HubCall::ConvexTables { .. }
        | HubCall::Convex { .. } => Ok(()),
        // The box's own run: its workflow session's callbacks, which the hub holds to the run it
        // works on, its delegated task's status, end and question back to the tab that delegated
        // it, its own task records and tabs, and a team's box asking the team's people.
        HubCall::Stage { .. }
        | HubCall::SlackReply { .. }
        | HubCall::LinearCreate { .. }
        | HubCall::LinearTake { .. }
        | HubCall::RunSet { .. }
        | HubCall::RemoteTask { .. }
        | HubCall::RemoteEnded { .. }
        | HubCall::RemoteAsked { .. }
        | HubCall::RecordTasks { .. }
        | HubCall::AutoArchive { .. }
        | HubCall::Team { .. } => Ok(()),
        // Writes: what the account's other machines load or run, and what its clients read.
        // The account's synced and shared files.
        HubCall::SyncPush { .. }
        | HubCall::SyncMarkExecutable { .. }
        | HubCall::ShareProject { .. }
        | HubCall::Clone { .. }
        // Its workflows, schedules and runs.
        | HubCall::Import { .. }
        | HubCall::SaveConnector { .. }
        | HubCall::DeleteConnector { .. }
        | HubCall::SaveSchedule { .. }
        | HubCall::PauseSchedule { .. }
        | HubCall::DeleteSchedule { .. }
        | HubCall::RunWorkflow { .. }
        | HubCall::GenerateWorkflow { .. }
        // Its way of working, which every session starts with.
        | HubCall::SetTaste { .. }
        | HubCall::ResetTasteField { .. }
        | HubCall::ImportTastes { .. }
        // Its MCP servers, which every machine loads into its sessions.
        | HubCall::SaveMcpServer { .. }
        | HubCall::EnableMcpServer { .. }
        | HubCall::RemoveMcpServer { .. }
        | HubCall::ImportMcpServer { .. }
        // Sessions on its other machines.
        | HubCall::Delegate { .. }
        | HubCall::Relay { .. }
        | HubCall::MarkSessions { .. }
        | HubCall::ArchiveSession { .. }
        | HubCall::UnarchiveSession { .. }
        | HubCall::DeleteSession { .. }
        // Its connections and settings.
        | HubCall::ConnectLinear { .. }
        | HubCall::DisconnectLinear
        | HubCall::ConnectSlack { .. }
        | HubCall::DisconnectSlack { .. }
        | HubCall::ConnectGithub { .. }
        | HubCall::DisconnectGithub
        | HubCall::ConnectVercel { .. }
        | HubCall::DisconnectVercel
        | HubCall::ConnectUpstash { .. }
        | HubCall::DisconnectUpstash
        | HubCall::ConnectConvex { .. }
        | HubCall::SetConvexRepos { .. }
        | HubCall::DisconnectConvex { .. }
        | HubCall::SetBoxAccess { .. }
        | HubCall::SetRepoInsight { .. }
        | HubCall::AnswerBox { .. }
        | HubCall::SetReleaseProtection { .. }
        | HubCall::SetBoxSize { .. }
        | HubCall::SetBoxClaudeToken { .. }
        | HubCall::RemoveBoxClaudeToken { .. }
        | HubCall::MintBoxClaudeToken { .. }
        | HubCall::CancelBoxClaudeToken { .. }
        // Its machines, boxes, people and agents.
        | HubCall::OpenBox { .. }
        | HubCall::ExtendBox { .. }
        | HubCall::RenameMachine { .. }
        | HubCall::RemoveMachine { .. }
        | HubCall::PairCode
        | HubCall::CreateInvite
        | HubCall::RevokeInvite { .. }
        | HubCall::AgentRequest { .. }
        | HubCall::ApproveAgent { .. }
        | HubCall::DenyAgent { .. }
        | HubCall::RevokeAgent { .. }
        | HubCall::DecideConfirmation { .. }
        | HubCall::Unlock { .. }
        // Its ideas and reports.
        | HubCall::SaveIdea { .. }
        | HubCall::DeleteIdea { .. }
        | HubCall::Reports { .. }
        | HubCall::ReportIssue { .. }
        // An agent's act: the call it carries is classed here too, where the hub admits it.
        | HubCall::Act { .. } => Err(BOX_REFUSAL.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::{box_may_send, BOX_REFUSAL};
    use pinned_core::sync;
    use pinned_core::wire::HubCall;

    fn call(value: serde_json::Value) -> HubCall {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_refusal_reads_as_the_accounts_size_limit_to_a_node() {
        assert_eq!(BOX_REFUSAL, "the account's synced projects would pass 0 MB from a box: a box only pulls the account's files");
        assert!(BOX_REFUSAL.starts_with(sync::LIMIT_REFUSAL));
    }

    #[test]
    fn a_box_may_only_pull_the_accounts_files() {
        let refused = Err("the account's synced projects would pass 0 MB from a box: a box only pulls the account's files".to_owned());
        let push = call(serde_json::json!({ "cmd": "sync_push", "project": "pinned:plugins", "files": [] }));
        assert_eq!(box_may_send(&push), refused);
        let mark = call(serde_json::json!({ "cmd": "sync_mark_executable", "project": "pinned:plugins", "repairs": [{ "path": "plugins/a/hooks/h.sh", "sha": "00" }] }));
        assert_eq!(box_may_send(&mark), refused);
        let share = call(serde_json::json!({ "cmd": "share_project", "project": { "key": "github.com/acme/site", "name": "site", "repo": "git@github.com:acme/site.git" } }));
        assert_eq!(box_may_send(&share), refused);
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "sync_list", "project": "pinned:plugins" }))), Ok(()));
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "sync_pull", "project": "pinned:plugins", "paths": ["user-skills/ship/SKILL.md"] }))), Ok(()));
    }
}
