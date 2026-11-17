//! A team's chat sessions on the hub. A session is a `team_runs` row of kind `session` bound to a
//! box of the team's account. A member reaches it only through `TeamOp::Session`: `team_scope`
//! read the membership for this op, the unlock check ran on the same call (`super::owner_only`),
//! and `session_rule` gates the personal `Call` the way `hub_rule` gates a `HubCall`. One door, so
//! what a member may do in a team session is reviewed here and nowhere else.
//!
//! Every member reads a session; only its driver changes it (its author, or the member who took it
//! over), and only while the owner has turned team sessions on for that member. A call names the
//! session's own tab and the hub puts in no other; a `Tabs` answer lists that tab only.

use super::{db, members_only, membership, Scope};
use crate::hub::{Hub, MACHINE_CALL_WAIT};
use pinned_core::surface;
use pinned_core::teams::{Member, Role, RunKind};
use pinned_core::wire::Call;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

/// What every refusal of the run says, so a bot's run, a run of another team, an id that is no id
/// and a session that does not exist cannot be told apart.
const NO_SUCH_SESSION: &str = "the team has no such session";

/// Who runs a personal call in a team session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionGate {
    /// Every member: it reads the session's own tab.
    Read,
    /// The session's driver, while the owner lets it drive team sessions: it changes the session.
    Drive,
    /// The team's owner only.
    Owner,
    /// Nobody: it belongs to a person's machine, account or terminal, or only the hub sends it.
    Never,
}

/// What a call names that must be the session's own.
enum Names<'a> {
    Tab(&'a str),
    /// A pull request's repository, which must be the session's project.
    Repo { host: &'a str, repo: &'a str },
    Nothing,
}

/// The team session's gate for `call`. Exhaustive on purpose, with no catch-all: a new `Call` does
/// not compile until it is placed here.
pub fn session_rule(call: &Call) -> SessionGate {
    placed(call).0
}

fn placed(call: &Call) -> (SessionGate, Names<'_>) {
    use SessionGate::{Drive, Never, Owner, Read};
    match call {
        Call::Thread { tab, .. }
        | Call::Workspace { tab }
        | Call::PullRequest { tab, .. }
        | Call::PrStatus { tab, .. }
        | Call::PrList { tab }
        | Call::PrGuide { tab, .. }
        | Call::PrCandidates { tab, .. }
        | Call::PrStack { tab, .. }
        | Call::Diff { tab, .. }
        | Call::Refs { tab }
        | Call::Branches { tab }
        | Call::Commands { tab }
        | Call::Subagent { tab, .. }
        | Call::OpenTask { tab, .. }
        | Call::ProjectScripts { tab }
        | Call::Favicon { tab }
        | Call::Ports { tab } => (Read, Names::Tab(tab)),
        // The tab list is cut to the session's tab after the box answers.
        Call::Tabs | Call::PageMeta { .. } | Call::ReadImage { .. } | Call::ReadUpload { .. } => (Read, Names::Nothing),
        Call::PullRequestFiles { host, repo, .. } => (Read, Names::Repo { host, repo }),
        Call::PullRequestStack { repo, .. } => (Read, Names::Repo { host: "github.com", repo }),
        // A model change that also switches the box's account is an account switch.
        Call::SetModel { account: Some(_), .. } => (Never, Names::Nothing),
        // Reading the box's files is the driver's too, as is every write to a pull request.
        Call::Files { tab }
        | Call::ReadFile { tab, .. }
        | Call::Search { tab, .. }
        | Call::Prompt { tab, .. }
        | Call::StopTurn { tab }
        | Call::AnswerQuestions { tab, .. }
        | Call::Wake { tab, .. }
        | Call::StartClaude { tab, .. }
        | Call::RestartClaude { tab, .. }
        | Call::SetModel { tab, account: None, .. }
        | Call::RegenerateTitle { tab, .. }
        | Call::Rewind { tab, .. }
        | Call::Git { tab, .. }
        | Call::Checkout { tab, .. }
        | Call::CheckoutPr { tab, .. }
        | Call::SetWorkspace { tab, .. }
        | Call::SaveFile { tab, .. }
        | Call::WriteFile { tab, .. }
        | Call::SaveProjectScripts { tab, .. }
        | Call::WatchPr { tab, .. }
        | Call::CancelTask { tab, .. }
        | Call::DeliverTask { tab, .. }
        | Call::PrAction { tab, .. }
        | Call::PrEdit { tab, .. }
        | Call::PrComment { tab, .. }
        | Call::PrReview { tab, .. }
        | Call::PrRerun { tab, .. }
        | Call::PrChange { tab, .. } => (Drive, Names::Tab(tab)),
        Call::Upload { .. } => (Drive, Names::Nothing),
        Call::SetBypass { tab, .. } => (Owner, Names::Tab(tab)),
        // A person's tabs and terminal: v1 is chat only, and a team box is not attachable.
        Call::OpenTab { .. }
        | Call::OpenScratch { .. }
        | Call::AutoResume { .. }
        | Call::CloseTab { .. }
        | Call::Fork { .. }
        | Call::WriteInput { .. }
        | Call::ResizePty { .. }
        | Call::OpenShell { .. }
        | Call::CloseShell { .. }
        | Call::BusyShells { .. }
        // A person's machine, its projects, folders and files outside the session.
        | Call::SetFolder { .. }
        | Call::ListProjects { .. }
        | Call::ProjectHistory { .. }
        | Call::GitRemote { .. }
        | Call::Browse { .. }
        | Call::CreateProject { .. }
        | Call::PublishProject { .. }
        | Call::Clone { .. }
        | Call::MakeFolder { .. }
        | Call::SearchThreads { .. }
        | Call::Run { .. }
        | Call::Editors { .. }
        | Call::OpenIn { .. }
        | Call::FileUrl { .. }
        | Call::Skills { .. }
        | Call::ReadSkill { .. }
        | Call::SaveSkill { .. }
        | Call::PullRequests { .. }
        | Call::PullRequestFilterOptions { .. }
        | Call::ReadSession { .. }
        | Call::WatchThreads { .. }
        | Call::MemorySearch { .. }
        | Call::MemorySave { .. }
        | Call::MemoryForget { .. }
        | Call::MemoryStatus { .. }
        // A person's accounts, usage and MCP servers.
        | Call::ListAccounts { .. }
        | Call::LearnModelMenu { .. }
        | Call::AddAccount { .. }
        | Call::SetAccountKey { .. }
        | Call::RemoveAccount { .. }
        | Call::SetProjectAccount { .. }
        | Call::SignIn { .. }
        | Call::SetupToken { .. }
        | Call::SwitchAccount { .. }
        | Call::TokenUsage { .. }
        | Call::UsagePrices { .. }
        | Call::SetUsagePrices { .. }
        | Call::UsageLimits { .. }
        | Call::McpServers { .. }
        | Call::McpImportable { .. }
        // A person's orchestrators, settings and devices, and the machine itself.
        | Call::OpenOrchestrator { .. }
        | Call::OpenGlobalOrchestrator { .. }
        | Call::OpenTasteChat { .. }
        | Call::GlobalOrchestratorSettings { .. }
        | Call::SetGlobalOrchestratorSettings { .. }
        | Call::AutoArchiveSettings { .. }
        | Call::SetAutoArchiveSettings { .. }
        | Call::SetBackground { .. }
        | Call::DeviceHub { .. }
        | Call::DeviceAgents { .. }
        | Call::Device { .. }
        | Call::UpdateSelf { .. }
        | Call::SyncStopped { .. }
        | Call::SyncLimit { .. }
        | Call::State { .. }
        | Call::ResetSync { .. }
        // What only the hub sends (`Call::only_the_hub_sends`).
        | Call::ActFor { .. }
        | Call::Notice { .. }
        | Call::RunNow { .. }
        | Call::OpenRun { .. }
        | Call::Deliver { .. }
        | Call::OrchestratorReport { .. }
        | Call::Delegate { .. }
        | Call::RemoteTask { .. }
        | Call::RemoteEnded { .. }
        | Call::RemoteAsked { .. }
        | Call::RepoLook { .. }
        | Call::Enqueue { .. }
        | Call::CancelSetupToken { .. } => (Never, Names::Nothing),
    }
}

/// Whether `call` names only what is the session's own: its `tab`, and its `project` for a pull
/// request's repository.
fn fits(call: &Call, tab: Option<&str>, project: Option<&str>) -> Result<(), String> {
    match placed(call).1 {
        Names::Tab(named) => match tab {
            None => Err("this session has no tab yet".into()),
            Some(own) if own == named => Ok(()),
            Some(_) => Err("that call names another tab than this session's".into()),
        },
        Names::Repo { host, repo } => match project {
            Some(project) if project.eq_ignore_ascii_case(&format!("{host}/{repo}")) => Ok(()),
            _ => Err("that call names another repository than this session's project".into()),
        },
        Names::Nothing => Ok(()),
    }
}

/// Whether a member with `role` runs a call of `gate` in a session: `drives` when the member is the
/// session's driver, `access` when the owner turned team sessions on for it.
fn allowed(gate: SessionGate, cmd: &str, role: Role, drives: bool, access: bool) -> Result<(), String> {
    match gate {
        SessionGate::Read => Ok(()),
        SessionGate::Drive if !drives => Err(format!("only the member driving this session does {cmd} in it")),
        SessionGate::Drive if role != Role::Owner && !access => Err("the team's owner has not turned on team sessions for you".into()),
        SessionGate::Drive => Ok(()),
        SessionGate::Owner if role == Role::Owner => Ok(()),
        SessionGate::Owner => Err(format!("only the team's owner does {cmd} in a team session")),
        SessionGate::Never => Err(format!("{cmd} does not run in a team session")),
    }
}

/// Runs a member's `call` in the team session `run`, on the box bound to it, if `session_rule` lets
/// the member. Only a live session of this team: a bot's run is refused as no session.
pub async fn session(hub: &Arc<Hub>, s: &Scope, run: &str, call: Call) -> Result<Value, String> {
    let (gate, cmd) = (session_rule(&call), surface::cmd_of(&call));
    if gate == SessionGate::Never {
        allowed(gate, &cmd, s.role, false, false)?;
    }
    let run_id = Uuid::parse_str(run.trim()).map_err(|_| NO_SUCH_SESSION)?;
    let session = db::run(&hub.db, s.team.id, run_id).await?.filter(|r| r.kind == RunKind::Session).ok_or(NO_SUCH_SESSION)?;
    if session.state.ended() {
        return Err("this session has ended".into());
    }
    fits(&call, session.tab.as_deref(), session.project.as_deref())?;
    let drives = session.controller.as_deref().unwrap_or(&session.started_by) == s.member.to_string();
    let access = gate == SessionGate::Drive && db::session_access(&hub.db, s.team.id, s.member).await?;
    allowed(gate, &cmd, s.role, drives, access)?;
    let machine = session.machine.ok_or("this session has no box yet")?;
    let tabs = matches!(call, Call::Tabs);
    let answer = hub.call_machine(&s.team, &machine, call, MACHINE_CALL_WAIT).await.map_err(String::from)?;
    Ok(if tabs { only_tab(answer, session.tab.as_deref()) } else { answer })
}

/// A `Tabs` answer cut to the session's own tab.
fn only_tab(answer: Value, tab: Option<&str>) -> Value {
    let tabs = answer.as_array().map(Vec::as_slice).unwrap_or_default();
    Value::Array(tabs.iter().filter(|t| tab.is_some_and(|tab| t["id"] == tab)).cloned().collect())
}

/// Owner only: whether `member` may open and drive team sessions. Answers the members.
pub async fn set_session_access(hub: &Arc<Hub>, s: &Scope, member: &str, on: bool) -> Result<Vec<Member>, String> {
    s.owner_only()?;
    let member = members_only(hub, s.team.id, &[member]).await?[0];
    if !db::set_session_access(&hub.db, s.team.id, member, on).await? {
        return Err(format!("no member {member} on this team"));
    }
    membership::members(hub, s).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn call(value: Value) -> Call {
        serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{value}: {e}"))
    }

    fn gates(calls: Vec<Value>, gate: SessionGate) {
        for value in calls {
            assert_eq!(session_rule(&call(value.clone())), gate, "{value}");
        }
    }

    #[test]
    fn every_member_reads_a_team_session_its_driver_changes_it_and_a_persons_machine_never_runs_in_it() {
        let t = "tab-1";
        gates(
            vec![
                json!({ "cmd": "tabs" }),
                json!({ "cmd": "thread", "tab": t, "from": 0 }),
                json!({ "cmd": "workspace", "tab": t }),
                json!({ "cmd": "pull_request", "tab": t, "pr": "7" }),
                json!({ "cmd": "pr_status", "tab": t, "pr": "7" }),
                json!({ "cmd": "pr_list", "tab": t }),
                json!({ "cmd": "pr_guide", "tab": t, "pr": "7" }),
                json!({ "cmd": "pr_candidates", "tab": t, "pr": "7", "of": "labels" }),
                json!({ "cmd": "pr_stack", "tab": t, "pr": "7", "action": "merge", "method": "squash", "heads": [{ "number": 7, "sha": "abc" }] }),
                json!({ "cmd": "diff", "tab": t, "scope": "base" }),
                json!({ "cmd": "refs", "tab": t }),
                json!({ "cmd": "branches", "tab": t }),
                json!({ "cmd": "commands", "tab": t }),
                json!({ "cmd": "subagent", "tab": t, "tool": "tool-1" }),
                json!({ "cmd": "open_task", "tab": t, "task": "t1" }),
                json!({ "cmd": "project_scripts", "tab": t }),
                json!({ "cmd": "favicon", "tab": t }),
                json!({ "cmd": "page_meta", "url": "http://127.0.0.1:3000" }),
                json!({ "cmd": "ports", "tab": t }),
                json!({ "cmd": "read_image", "path": "/tmp/shots/app3.png" }),
                json!({ "cmd": "read_upload", "path": "uploads/shot.png" }),
                json!({ "cmd": "pull_request_files", "host": "github.com", "repo": "acme/site", "number": 7 }),
                json!({ "cmd": "pull_request_stack", "repo": "acme/site", "number": 7 }),
            ],
            SessionGate::Read,
        );
        gates(
            vec![
                // What reads the box's files is the driver's too, as is every write to a pull request.
                json!({ "cmd": "files", "tab": t }),
                json!({ "cmd": "read_file", "tab": t, "path": "src/a.rs" }),
                json!({ "cmd": "search", "tab": t, "query": "footer" }),
                json!({ "cmd": "prompt", "tab": t, "text": "Fix the footer" }),
                json!({ "cmd": "stop_turn", "tab": t }),
                json!({ "cmd": "answer_questions", "tab": t, "id": "q-7", "reply": { "kind": "skipped" } }),
                json!({ "cmd": "wake", "tab": t, "cols": 80, "rows": 24 }),
                json!({ "cmd": "start_claude", "tab": t, "cols": 80, "rows": 24 }),
                json!({ "cmd": "restart_claude", "tab": t, "cols": 80, "rows": 24 }),
                json!({ "cmd": "set_model", "tab": t, "model": "opus", "cols": 80, "rows": 24 }),
                json!({ "cmd": "regenerate_title", "tab": t, "previous": "Footer", "thread": "Fix the footer" }),
                json!({ "cmd": "upload", "name": "photo.jpg", "data": "/9g=" }),
                json!({ "cmd": "rewind", "tab": t, "before": "u2", "files": false, "cols": 80, "rows": 24 }),
                json!({ "cmd": "git", "tab": t, "step": { "step": "push" } }),
                json!({ "cmd": "checkout", "tab": t, "branch": "main" }),
                json!({ "cmd": "checkout_pr", "tab": t, "pr": "7" }),
                json!({ "cmd": "set_workspace", "tab": t, "worktree": null, "cols": 80, "rows": 24 }),
                json!({ "cmd": "save_file", "tab": t, "path": "a.rs", "text": "" }),
                json!({ "cmd": "write_file", "tab": t, "path": "a.rs", "text": "" }),
                json!({ "cmd": "save_project_scripts", "tab": t, "scripts": [] }),
                json!({ "cmd": "watch_pr", "tab": t, "pr": "7", "watching": true }),
                json!({ "cmd": "cancel_task", "tab": t, "task": "t1" }),
                json!({ "cmd": "deliver_task", "tab": t, "task": "t1" }),
                json!({ "cmd": "pr_action", "tab": t, "pr": "7", "action": "merge" }),
                json!({ "cmd": "pr_edit", "tab": t, "pr": "7", "title": "Footer" }),
                json!({ "cmd": "pr_comment", "tab": t, "pr": "7", "body": "Looks good" }),
                json!({ "cmd": "pr_review", "tab": t, "pr": "7", "event": "APPROVE" }),
                json!({ "cmd": "pr_rerun", "tab": t, "check": "ci" }),
                json!({ "cmd": "pr_change", "tab": t, "pr": "7", "change": { "kind": "reviewer", "login": "acme/core", "team": true, "on": false } }),
            ],
            SessionGate::Drive,
        );
        gates(vec![json!({ "cmd": "set_bypass", "tab": t, "on": true, "cols": 80, "rows": 24 })], SessionGate::Owner);
        gates(
            vec![
                // A person's tabs and terminal.
                json!({ "cmd": "open_tab", "path": null, "bypass": false }),
                json!({ "cmd": "open_scratch", "bypass": false }),
                json!({ "cmd": "auto_resume", "tab": t, "on": true }),
                json!({ "cmd": "close_tab", "tab": t }),
                json!({ "cmd": "fork", "tab": t, "through": "u2" }),
                json!({ "cmd": "write_input", "tab": t, "data": "ls\n" }),
                json!({ "cmd": "resize_pty", "tab": t, "cols": 80, "rows": 24 }),
                json!({ "cmd": "open_shell", "tab": t, "cols": 80, "rows": 24 }),
                json!({ "cmd": "close_shell", "shell": "s1" }),
                json!({ "cmd": "busy_shells", "shells": [] }),
                // A person's machine, projects and files outside the session.
                json!({ "cmd": "set_folder", "tab": t, "path": "/w" }),
                json!({ "cmd": "list_projects" }),
                json!({ "cmd": "browse", "path": "/" }),
                json!({ "cmd": "clone", "repo": "acme/site" }),
                json!({ "cmd": "make_folder", "path": "/w/x" }),
                json!({ "cmd": "search_threads", "query": "footer" }),
                json!({ "cmd": "file_url", "tab": t, "path": "a.rs" }),
                json!({ "cmd": "open_in", "tab": t, "editor": "code" }),
                json!({ "cmd": "editors" }),
                json!({ "cmd": "skills" }),
                json!({ "cmd": "read_session", "tab": t }),
                json!({ "cmd": "memory_status", "path": "/w" }),
                json!({ "cmd": "update_self" }),
                json!({ "cmd": "state" }),
                json!({ "cmd": "watch_threads", "tabs": [t] }),
                json!({ "cmd": "token_usage", "since": 0 }),
                json!({ "cmd": "pull_requests", "paths": [], "state": "open", "involvement": "all" }),
                // A person's accounts.
                json!({ "cmd": "list_accounts" }),
                json!({ "cmd": "switch_account", "tab": t, "account": "claude", "cols": 80, "rows": 24 }),
                json!({ "cmd": "sign_in", "account": "claude" }),
                json!({ "cmd": "setup_token", "account": "claude" }),
                json!({ "cmd": "set_account_key", "account": "claude", "key": "k" }),
                json!({ "cmd": "remove_account", "account": "claude" }),
                json!({ "cmd": "mcp_servers" }),
                // What only the hub sends.
                json!({ "cmd": "notice", "tab": t, "text": "hi", "id": "n1" }),
                json!({ "cmd": "act_for", "caller": t, "call": { "cmd": "tabs" } }),
                json!({ "cmd": "deliver", "env": [], "text": "hi" }),
                json!({ "cmd": "cancel_setup_token", "account": "claude" }),
                json!({ "cmd": "remote_task", "origin_machine": "m1", "tab": t, "task": "t1" }),
            ],
            SessionGate::Never,
        );
    }

    #[test]
    fn a_session_call_names_the_sessions_own_tab_and_its_project_only() {
        let (tab, project) = (Some("tab-1"), Some("github.com/acme/site"));
        let fits = |value: Value| fits(&call(value), tab, project);
        assert_eq!(fits(json!({ "cmd": "prompt", "tab": "tab-1", "text": "hi" })), Ok(()));
        assert_eq!(fits(json!({ "cmd": "prompt", "tab": "tab-2", "text": "hi" })), Err("that call names another tab than this session's".into()));
        assert_eq!(fits(json!({ "cmd": "tabs" })), Ok(()));
        assert_eq!(fits(json!({ "cmd": "pull_request_files", "host": "github.com", "repo": "acme/site", "number": 7 })), Ok(()));
        assert_eq!(fits(json!({ "cmd": "pull_request_stack", "repo": "acme/site", "number": 7 })), Ok(()));
        let other = Err("that call names another repository than this session's project".to_owned());
        assert_eq!(fits(json!({ "cmd": "pull_request_files", "host": "github.com", "repo": "acme/vault", "number": 7 })), other);
        assert_eq!(fits(json!({ "cmd": "pull_request_files", "host": "example.test", "repo": "acme/site", "number": 7 })), other);
        assert_eq!(fits(json!({ "cmd": "pull_request_stack", "repo": "acme/vault", "number": 7 })), other);
        // A session whose box has not reported its tab yet takes no call that names one.
        assert_eq!(super::fits(&call(json!({ "cmd": "thread", "tab": "tab-1", "from": 0 })), None, project), Err("this session has no tab yet".into()));
    }
}
