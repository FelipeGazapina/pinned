//! What the hub takes from a box machine of a person's account. A box is a machine of the account
//! that opened it, and the account's other machines load and run what reaches its synced and
//! shared files, so a box may only pull the account's files: the hub refuses every call of a box
//! that would change what the account's other machines or clients read.

use crate::hub::Account;
use pinned_core::global_orchestrator::RelayTo;
use pinned_core::surface::Parsed;
use pinned_core::wire::HubCall;
use uuid::Uuid;

/// The refusal a box's sync push gets. It starts with `sync::LIMIT_REFUSAL`, so a node that reads a
/// refused push as the account's size limit backs off its push and still pulls in the same cycle.
pub const SYNC_REFUSAL: &str = "the account's synced projects would pass 0 MB from a box: a box only pulls the account's files";

/// The refusal every other call or act of a box gets: what of the account it may not change.
pub fn refusal(what: &str) -> String {
    format!("a box may not change the account's {what}")
}

const SHARED: &str = "shared projects";
const WORKFLOWS: &str = "workflows";
const TASTES: &str = "way of working";
const MCP: &str = "MCP servers";
const SESSIONS: &str = "sessions on its other machines";
const SETTINGS: &str = "settings";
const MACHINES: &str = "machines";
const PEOPLE: &str = "people and agents";
const IDEAS: &str = "ideas and reports";

/// Whether the machine is held to this gate: it signed in as a box on the connection a call came
/// on (`boxed`), or its live connection did. Every check of the gate asks this one.
pub fn is_box(boxed: bool, account: &Account, machine: &str) -> bool {
    boxed || account.dedicated(machine)
}

/// Whether a box machine may send `call`: a read, or a callback of the box's own run. Every call
/// is classed here by name, so a call added later is classed by whoever adds it.
pub fn box_may_send(call: &HubCall) -> Result<(), String> {
    // No wildcard arm, on purpose: a new `HubCall` fails to compile until someone decides whether a box may send it.
    let refused = match call {
        // Reads: what the account has, and what its machines report.
        HubCall::GetSettings
        | HubCall::LinearSuggestions
        | HubCall::SlackSuggestions { .. }
        | HubCall::GithubRepos
        | HubCall::VercelModels
        | HubCall::SharedProjects
        | HubCall::SyncList { .. }
        | HubCall::SyncChanges { .. }
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
        | HubCall::Convex { .. } => return Ok(()),
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
        | HubCall::Team { .. } => return Ok(()),
        // A Capitão's report on its own work, and a worker it hands to another box, which is held
        // to this gate as this box is.
        HubCall::Relay { to: RelayTo::Orchestrator { .. }, .. } => return Ok(()),
        HubCall::Delegate { request, .. } if request.target.boxed.is_some() => return Ok(()),
        // Writes: what the account's other machines load or run, and what its clients read.
        // The account's synced files, refused as the size limit is (`SYNC_REFUSAL`).
        HubCall::SyncPush { .. } | HubCall::SyncMarkExecutable { .. } => return Err(SYNC_REFUSAL.to_owned()),
        // Its shared projects.
        HubCall::ShareProject { .. } | HubCall::Clone { .. } => SHARED,
        // Its workflows, schedules and runs.
        HubCall::Import { .. }
        | HubCall::SaveConnector { .. }
        | HubCall::DeleteConnector { .. }
        | HubCall::SaveSchedule { .. }
        | HubCall::PauseSchedule { .. }
        | HubCall::DeleteSchedule { .. }
        | HubCall::RunWorkflow { .. }
        | HubCall::GenerateWorkflow { .. } => WORKFLOWS,
        // Its way of working, which every session starts with.
        HubCall::SetTaste { .. }
        | HubCall::ResetTasteField { .. }
        | HubCall::ImportTastes { .. }
        | HubCall::SetReviewer { .. }
        | HubCall::SetWorkerProfiles { .. }
        | HubCall::ProposeWorkerProfile { .. } => TASTES,
        // Its MCP servers, which every machine loads into its sessions.
        HubCall::SaveMcpServer { .. } | HubCall::EnableMcpServer { .. } | HubCall::RemoveMcpServer { .. } | HubCall::ImportMcpServer { .. } => MCP,
        // Sessions on its other machines: a worker there, a Capitão's messages.
        HubCall::Delegate { .. }
        | HubCall::Relay { .. }
        | HubCall::MarkSessions { .. }
        | HubCall::ArchiveSession { .. }
        | HubCall::UnarchiveSession { .. }
        | HubCall::DeleteSession { .. } => SESSIONS,
        // Its connections and settings.
        HubCall::ConnectLinear { .. }
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
        | HubCall::CancelBoxClaudeToken { .. } => SETTINGS,
        // Its machines and boxes. A box's session asks to extend its own box through `box_may_act`.
        HubCall::OpenBox { .. } | HubCall::ExtendBox { .. } | HubCall::RenameMachine { .. } | HubCall::RemoveMachine { .. } | HubCall::PairCode => MACHINES,
        // Its people and agents.
        HubCall::CreateInvite
        | HubCall::RevokeInvite { .. }
        | HubCall::AgentRequest { .. }
        | HubCall::ApproveAgent { .. }
        | HubCall::DenyAgent { .. }
        | HubCall::RevokeAgent { .. }
        | HubCall::DecideConfirmation { .. }
        | HubCall::Unlock { .. }
        // An agent's act: the call it carries is classed here too, where the hub admits it.
        | HubCall::Act { .. } => PEOPLE,
        // Its ideas and reports.
        HubCall::SaveIdea { .. } | HubCall::DeleteIdea { .. } | HubCall::Reports { .. } | HubCall::ReportIssue { .. } => IDEAS,
    };
    Err(refusal(refused))
}

/// Whether a box's session may act `call` from `origin`, the box: what `box_may_send` lets the box
/// send, a read on another machine, anything on the box itself, and its ask to extend the box
/// itself, which `extend_box` holds for the owner to decide (`surface::held_for_owner`).
pub fn box_may_act(origin: &str, call: &Parsed, on_origin: bool, reads: bool) -> Result<(), String> {
    match call {
        Parsed::Hub(HubCall::ExtendBox { machine, .. }) if same_machine(machine, origin) => Ok(()),
        Parsed::Hub(call) => box_may_send(call),
        Parsed::Node(_) if !on_origin && !reads => Err(refusal(MACHINES)),
        Parsed::Node(_) => Ok(()),
    }
}

fn same_machine(named: &str, origin: &str) -> bool {
    matches!((Uuid::parse_str(named.trim()), Uuid::parse_str(origin)), (Ok(named), Ok(origin)) if named == origin)
}

#[cfg(test)]
mod tests {
    use super::{box_may_act, box_may_send, refusal, SYNC_REFUSAL};
    use pinned_core::surface::Parsed;
    use pinned_core::sync;
    use pinned_core::wire::{Call, HubCall};

    fn call(value: serde_json::Value) -> HubCall {
        serde_json::from_value(value).unwrap()
    }

    const BOX: &str = "6f1c0b1e-0000-4000-8000-0000000000b0";
    const MAC: &str = "6f1c0b1e-0000-4000-8000-0000000000a0";

    #[test]
    fn only_a_refused_sync_reads_as_the_accounts_size_limit_to_a_node() {
        assert_eq!(SYNC_REFUSAL, "the account's synced projects would pass 0 MB from a box: a box only pulls the account's files");
        assert!(SYNC_REFUSAL.starts_with(sync::LIMIT_REFUSAL));
        assert_eq!(refusal("settings"), "a box may not change the account's settings");
        assert!(!refusal("settings").starts_with(sync::LIMIT_REFUSAL));
    }

    #[test]
    fn a_box_may_only_pull_the_accounts_files() {
        let refused = Err("the account's synced projects would pass 0 MB from a box: a box only pulls the account's files".to_owned());
        let push = call(serde_json::json!({ "cmd": "sync_push", "project": "pinned:plugins", "files": [] }));
        assert_eq!(box_may_send(&push), refused);
        let mark = call(serde_json::json!({ "cmd": "sync_mark_executable", "project": "pinned:plugins", "repairs": [{ "path": "plugins/a/hooks/h.sh", "sha": "00" }] }));
        assert_eq!(box_may_send(&mark), refused);
        let share = call(serde_json::json!({ "cmd": "share_project", "project": { "key": "github.com/acme/site", "name": "site", "repo": "git@github.com:acme/site.git" } }));
        assert_eq!(box_may_send(&share), Err("a box may not change the account's shared projects".to_owned()));
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "sync_list", "project": "pinned:plugins" }))), Ok(()));
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "sync_pull", "project": "pinned:plugins", "paths": ["user-skills/ship/SKILL.md"] }))), Ok(()));
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "save_idea", "title": "An iPad app" }))), Err("a box may not change the account's ideas and reports".to_owned()));
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "set_taste", "scope": { "kind": "orchestrator" }, "patch": {} }))), Err("a box may not change the account's way of working".to_owned()));
    }

    #[test]
    fn a_box_may_not_set_the_reviewer_or_the_worker_profiles() {
        let refused = Err(refusal("way of working"));
        assert_eq!(refused, Err("a box may not change the account's way of working".to_owned()));
        let profile = serde_json::json!({ "name": "reviewer-lite" });
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "set_reviewer", "patch": { "rounds": 2 } }))), refused);
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "set_worker_profiles", "profiles": [profile.clone()] }))), refused);
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "propose_worker_profile", "tab": "tab-1", "profile": profile }))), refused);
    }

    #[test]
    fn a_box_hands_a_worker_only_to_another_box() {
        let delegate = |target: serde_json::Value| {
            let request = serde_json::to_value(pinned_core::delegate::parse_request(&serde_json::json!({ "task": "Fix it.", "target": target })).unwrap()).unwrap();
            call(serde_json::json!({ "cmd": "delegate", "tab": "tab-1", "task": "t1", "request": request, "access": "auto" }))
        };
        assert_eq!(box_may_send(&delegate(serde_json::json!({ "box": true }))), Ok(()));
        assert_eq!(box_may_send(&delegate(serde_json::json!({ "machine": "mac" }))), Err("a box may not change the account's sessions on its other machines".to_owned()));
    }

    #[test]
    fn a_box_still_asks_to_extend_itself_and_only_itself() {
        let extend = |machine: &str| Parsed::Hub(call(serde_json::json!({ "cmd": "extend_box", "machine": machine, "extra_usd": 1.0 })));
        assert_eq!(box_may_act(BOX, &extend(BOX), true, false), Ok(()));
        assert_eq!(box_may_act(BOX, &extend(&BOX.to_uppercase()), true, false), Ok(()), "the id as any case names the same box");
        let machines = Err("a box may not change the account's machines".to_owned());
        assert_eq!(box_may_act(BOX, &extend(MAC), true, false), machines);
        assert_eq!(box_may_act(BOX, &extend("vps"), true, false), machines);
        assert_eq!(box_may_send(&call(serde_json::json!({ "cmd": "extend_box", "machine": BOX, "extra_usd": 1.0 }))), machines, "only as a session's ask");
    }

    /// The call an agent's act of `action` with `args` parses to, as the hub holds it to the gate.
    fn acted(action: &str, args: serde_json::Value) -> Parsed {
        pinned_core::surface::parse(pinned_core::surface::find(action).unwrap(), &args).unwrap()
    }

    #[test]
    fn a_boxs_agent_may_not_save_an_mcp_server_import_one_save_a_connector_or_set_a_taste() {
        let server = serde_json::json!({ "name": "ctx", "transport": { "type": "stdio", "command": "npx" } });
        let connector = serde_json::json!({
            "name": "Nightly", "enabled": false, "trigger": { "kind": "linear-label", "label": "agent" },
            "folder": "/w/site", "model": "claude-sonnet-5-5", "effort": "medium", "permissionMode": "acceptEdits", "worktree": false, "instructions": "Check the build.",
        });
        let acts = [
            (acted("save_mcp_server", serde_json::json!({ "server": server })), "a box may not change the account's MCP servers"),
            (acted("import_mcp_server", serde_json::json!({ "machine": MAC, "account": "claude", "name": "linear" })), "a box may not change the account's MCP servers"),
            (acted("save_connector", serde_json::json!({ "connector": connector })), "a box may not change the account's workflows"),
            (acted("set_taste", serde_json::json!({ "scope": { "kind": "orchestrator" }, "patch": {} })), "a box may not change the account's way of working"),
        ];
        for (act, refused) in acts {
            assert_eq!(box_may_act(BOX, &act, true, false), Err(refused.to_owned()), "on the box itself");
            assert_eq!(box_may_act(BOX, &act, false, false), Err(refused.to_owned()), "on another machine");
        }
    }

    #[test]
    fn a_boxs_node_act_on_a_persons_machine_is_refused_when_it_writes_and_passes_when_it_reads() {
        let write = acted("make_folder", serde_json::json!({ "path": "/w/new" }));
        let read = acted("list_projects", serde_json::Value::Null);
        assert!(matches!((&write, &read), (Parsed::Node(_), Parsed::Node(_))));
        assert_eq!(box_may_act(BOX, &write, false, false), Err("a box may not change the account's machines".to_owned()));
        assert_eq!(box_may_act(BOX, &read, false, true), Ok(()));
        assert_eq!(box_may_act(BOX, &write, true, false), Ok(()), "a write on the box itself is its own");
    }

    #[test]
    fn a_box_acts_on_another_machine_only_to_read() {
        let node = Parsed::Node(Call::SyncLimit);
        assert_eq!(box_may_act(BOX, &node, true, false), Ok(()), "on the box itself");
        assert_eq!(box_may_act(BOX, &node, false, true), Ok(()));
        assert_eq!(box_may_act(BOX, &node, false, false), Err("a box may not change the account's machines".to_owned()));
    }
}
