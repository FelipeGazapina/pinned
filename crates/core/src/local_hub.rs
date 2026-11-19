//! What this machine answers itself: from its own store when signed out, and from its copy of the
//! account when signed in. The desktop's hook listener calls [`signed_out`]. [`LocalHub::answer`]
//! is the one dispatcher for every [`crate::wire::HubCall`].

use crate::agent::{Message, Stage, Subject};
use crate::connectors::Connectors;
use crate::wire::HubCall;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

/// What `ask_people` answers when no hub can take the question.
pub const NEEDS_THE_HUB: &str = "needs the hub";

/// What a feature that lives only on the hub answers while it is off.
pub const NEEDS_THE_HUB_OFF: &str = "needs the hub, which is off";

const SIGN_IN_MCP: &str = "sign in to keep MCP servers";
const BOX_SIGNED_OUT: &str = "this workflow runs in an Upstash Box, which needs pinned signed in";
const BOX_OFF: &str = "Upstash Boxes need the hub, which is off";

/// What only a signed-in copy has, beyond the connectors store.
pub trait AccountSide: Send + Sync {
    /// Keeps a `pinned run set` for a run the hub owns until the hub takes it.
    fn keep_run_set(&self, set: crate::account_copy::RunSet);
}

/// The store, the machine, and when signed in the copy's taste file, that answer a hub call here.
pub struct LocalHub<'a> {
    pub connectors: &'a Connectors,
    pub machine: &'a Arc<crate::machine::Machine>,
    /// `local` signed out. The machine id once this machine runs the account's copy.
    pub place_as: &'a str,
    /// The copy's taste file. None reads and writes this machine's own.
    pub taste_dir: Option<&'a Path>,
    pub account: Option<&'a dyn AccountSide>,
}

/// What [`LocalHub::answer`] decided.
#[derive(Debug, PartialEq)]
pub enum Answer {
    Done(Result<Value, String>),
    /// Only the hub answers this. The caller keeps its own offline error.
    HubOnly,
}

fn done<T: Serialize>(result: Result<T, String>) -> Answer {
    Answer::Done(result.and_then(|value| serde_json::to_value(value).map_err(|e| e.to_string())))
}

fn project_repo(repo: &str) -> Result<String, String> {
    if let Some(key) = crate::machine::repo_key(repo) {
        return Ok(key);
    }
    let parts: Vec<&str> = repo.split('/').filter(|part| !part.is_empty()).collect();
    if parts.len() == 2 {
        if let Some(key) = crate::machine::repo_key(&format!("https://github.com/{}/{}.git", parts[0], parts[1])) {
            return Ok(key);
        }
    }
    Err(format!("{repo} names no repository: use owner/repo or its clone URL"))
}

/// A callback the account handed back, answered here or left for the desktop's connectors.
#[derive(Debug, PartialEq)]
pub enum SignedOut {
    Done(Result<String, String>),
    Stage {
        connector: String,
        subject: Subject,
        stage: Stage,
    },
    SlackReply {
        connector: String,
        channel: String,
        thread: String,
        text: String,
    },
    LinearCreate {
        connector: String,
        channel: String,
        thread: String,
        title: String,
        description: String,
        assignee: String,
    },
    LinearTake {
        connector: String,
        channel: String,
        thread: String,
        issue: String,
    },
}

/// Answers a signed-out hook message. `RunSet` goes to the store's live run. `AskPeople` cannot.
pub fn signed_out(connectors: &Connectors, message: Message) -> SignedOut {
    match message {
        Message::Stage {
            connector,
            subject,
            stage,
        } => SignedOut::Stage {
            connector,
            subject,
            stage,
        },
        Message::SlackReply {
            connector,
            channel,
            thread,
            text,
        } => SignedOut::SlackReply {
            connector,
            channel,
            thread,
            text,
        },
        Message::LinearCreate {
            connector,
            channel,
            thread,
            title,
            description,
            assignee,
        } => SignedOut::LinearCreate {
            connector,
            channel,
            thread,
            title,
            description,
            assignee,
        },
        Message::LinearTake {
            connector,
            channel,
            thread,
            issue,
        } => SignedOut::LinearTake {
            connector,
            channel,
            thread,
            issue,
        },
        Message::RunSet {
            connector,
            subject,
            name,
            value,
        } => SignedOut::Done(connectors.run_set(&connector, &subject, &name, &value)),
        Message::AskPeople { .. } => SignedOut::Done(Err(NEEDS_THE_HUB.into())),
        Message::Agent(_)
        | Message::Compacting { .. }
        | Message::Team { .. }
        | Message::Remind { .. }
        | Message::ListReminders
        | Message::CancelReminder { .. }
        | Message::Project { .. }
        | Message::Guard { .. }
        | Message::Usage { .. }
        | Message::RefreshUsage { .. }
        | Message::Delegate { .. }
        | Message::TaskStatus { .. }
        | Message::TaskCancel { .. }
        | Message::Capabilities { .. }
        | Message::TaskReview { .. }
        | Message::DelegationStats { .. } => SignedOut::Done(Err(
            "only a workflow session's callbacks go to the hub".into(),
        )),
    }
}

impl LocalHub<'_> {
    fn tastes_at(&self) -> Option<&Path> {
        self.taste_dir.or_else(|| self.machine.taste_dir())
    }

    fn read_tastes(&self) -> crate::taste::Tastes {
        crate::taste::load(self.tastes_at()).unwrap_or_default()
    }

    fn change_tastes(&self, change: impl FnOnce(&mut crate::taste::Tastes) -> Result<(), String>) -> Result<crate::taste::Tastes, String> {
        crate::taste::change(self.tastes_at(), change)
    }

    fn on_this_machine(&self, machine: &str) -> bool {
        machine.is_empty() || machine == "local" || machine == self.place_as || machine == self.machine.listed_id()
    }

    /// Every `HubCall` is named here, so a new one does not compile until it is classed.
    pub async fn answer(&self, call: HubCall) -> Answer {
        use crate::connectors::{off_worker, run_now_title, RunsOn};
        use crate::taste::{TasteField, TastePatch, TasteScope};
        use crate::wire::Call;
        match call {
            HubCall::GetSettings => done(Ok(self.connectors.get_settings())),
            HubCall::ConnectLinear { key } => done(self.connectors.connect_linear(&key).await),
            HubCall::DisconnectLinear => done(self.connectors.disconnect_linear()),
            HubCall::LinearSuggestions => done(self.connectors.linear_suggestions().await),
            HubCall::ConnectSlack { token } => done(self.connectors.connect_slack(&token).await),
            HubCall::DisconnectSlack { bot } => done(self.connectors.disconnect_slack(&bot)),
            HubCall::SlackSuggestions { bot } => done(self.connectors.slack_suggestions(&bot).await),
            HubCall::ConnectGithub { token } => done(self.connectors.connect_github(&token).await),
            HubCall::DisconnectGithub => done(self.connectors.disconnect_github()),
            HubCall::GithubRepos => done(self.connectors.github_repos().await),
            HubCall::ConnectVercel { key } => done(self.connectors.connect_vercel(&key).await),
            HubCall::DisconnectVercel => done(self.connectors.disconnect_vercel()),
            HubCall::VercelModels => done(self.connectors.vercel_models().await),
            HubCall::SaveConnector { connector } => done(self.connectors.save_connector(connector)),
            HubCall::DeleteConnector { id } => done(self.connectors.delete_connector(&id)),
            HubCall::SetRepoInsight { settings } => done(self.connectors.set_repo_insight(settings)),
            HubCall::RunWorkflow { id } => {
                let run = match self.connectors.run_now(&id, "the account's owner, from Settings › Workflows") {
                    Ok(run) => run,
                    Err(error) => return Answer::Done(Err(error)),
                };
                if matches!(run.runs_on, RunsOn::FreshBox(_)) {
                    let message = if self.place_as == "local" { BOX_SIGNED_OUT } else { BOX_OFF };
                    return Answer::Done(Err(message.into()));
                }
                let spec = run.spec();
                let title = run_now_title(&spec);
                let machine = Arc::clone(self.machine);
                let tab = match off_worker(move || machine.run_now(&spec, &title, None)).await {
                    Ok(tab) => tab,
                    Err(error) => return Answer::Done(Err(error)),
                };
                done(Ok(crate::wire::RunStarted { workflow: id, machine: self.place_as.to_owned(), machine_name: String::new(), tab }))
            }
            HubCall::Clone { machine, repo, dest } => {
                if self.account.is_some() && !self.on_this_machine(&machine) {
                    return Answer::HubOnly;
                }
                let repo = crate::github::clone_url(&repo);
                let token = self.connectors.github_token().filter(|_| crate::github::hosts(&repo));
                let machine = Arc::clone(self.machine);
                match off_worker(move || machine.call(Call::Clone { repo, token, dest })).await {
                    Ok(value) => Answer::Done(Ok(value)),
                    Err(error) => Answer::Done(Err(error)),
                }
            }
            HubCall::Stage { connector, subject, stage } => done(self.connectors.stage(&connector, &subject, stage).await),
            HubCall::SlackReply { connector, channel, thread, text } => done(self.connectors.slack_reply(&connector, &channel, &thread, &text).await),
            HubCall::LinearCreate { connector, channel, thread, title, description, assignee } => {
                done(self.connectors.linear_create(&connector, &channel, &thread, &title, &description, &assignee).await)
            }
            HubCall::LinearTake { connector, channel, thread, issue } => done(self.connectors.linear_take(&connector, &channel, &thread, &issue).await),
            HubCall::RunSet { connector, subject, name, value } => match self.account {
                Some(account) if !self.connectors.has_run(&connector, &subject) => {
                    let shown = name.clone();
                    account.keep_run_set(crate::account_copy::RunSet { connector, subject, name, value });
                    done(Ok(format!("kept {{{{vars.{shown}}}}} until the hub takes this run")))
                }
                _ => done(self.connectors.run_set(&connector, &subject, &name, &value)),
            },
            HubCall::GetTastes => done(Ok(self.read_tastes())),
            HubCall::SetTaste { scope, patch } => done(self.change_tastes(|tastes| tastes.set(&scope, patch))),
            HubCall::ResetTasteField { scope, field } => done(self.change_tastes(|tastes| {
                tastes.reset(&scope, field);
                Ok(())
            })),
            HubCall::SetReviewer { patch } => done(self.change_tastes(|tastes| tastes.set_reviewer(patch))),
            HubCall::SetWorkerProfiles { project, profiles, drop_proposed } => done(self.change_tastes(|tastes| {
                tastes.set_worker_profiles(project.as_deref(), profiles)?;
                tastes.drop_proposed(&drop_proposed);
                Ok(())
            })),
            HubCall::ProposeWorkerProfile { profile, .. } => done(self.change_tastes(|tastes| tastes.propose_worker_profile(profile))),
            HubCall::ImportTastes { tastes } => {
                if let Err(error) = tastes.check() {
                    return Answer::Done(Err(error));
                }
                if self.read_tastes() != crate::taste::Tastes::default() {
                    return done(Ok(false));
                }
                match crate::taste::save(&tastes, self.tastes_at()) {
                    Ok(()) => done(Ok(true)),
                    Err(error) => Answer::Done(Err(error)),
                }
            }
            HubCall::GetBoxSize { repo, size } => done(Ok(self.read_tastes().box_size(repo.as_deref(), size))),
            HubCall::SetBoxSize { repo, size } => {
                let scope = match repo {
                    Some(repo) => match project_repo(&repo) {
                        Ok(repo) => TasteScope::Project { repo },
                        Err(error) => return Answer::Done(Err(error)),
                    },
                    None => TasteScope::Captains,
                };
                done(self.change_tastes(|tastes| match size {
                    Some(size) => tastes.set(&scope, TastePatch { box_size: Some(size), ..TastePatch::default() }),
                    None => {
                        tastes.reset(&scope, TasteField::BoxSize);
                        Ok(())
                    }
                }))
            }
            HubCall::ListMcpServers | HubCall::SaveMcpServer { .. } | HubCall::EnableMcpServer { .. } | HubCall::RemoveMcpServer { .. } | HubCall::LoadMcpServers | HubCall::ImportMcpServer { .. } if self.account.is_none() => {
                Answer::Done(Err(SIGN_IN_MCP.into()))
            }
            HubCall::ListMcpServers => done(Ok(self.connectors.mcp_servers())),
            HubCall::SaveMcpServer { id, server, secrets } => done(self.connectors.save_mcp_server(id.as_deref(), server, &secrets)),
            HubCall::EnableMcpServer { id, enabled } => done(self.connectors.enable_mcp_server(&id, enabled)),
            HubCall::RemoveMcpServer { id } => done(self.connectors.remove_mcp_server(&id)),
            HubCall::LoadMcpServers => done(Ok(self.connectors.load_mcp_servers())),
            HubCall::ImportMcpServer { machine, account, name, path, project, secrets } => {
                if !self.on_this_machine(&machine) {
                    return Answer::HubOnly;
                }
                let here = Arc::clone(self.machine);
                let account_name = account.clone();
                let found = match off_worker(move || here.call(Call::McpImportable { account: Some(account_name), path })).await {
                    Ok(found) => found,
                    Err(error) => return Answer::Done(Err(error)),
                };
                let found: Vec<crate::mcp::Found> = match serde_json::from_value(found) {
                    Ok(found) => found,
                    Err(error) => return Answer::Done(Err(format!("this Mac answered servers that did not read: {error}"))),
                };
                let Some(found) = found.into_iter().find(|server| server.server.name == name) else {
                    return Answer::Done(Err(format!("Claude Code account {account} on this Mac has no MCP server {name}")));
                };
                let server = crate::mcp::Spec { project, ..found.server };
                done(self.connectors.save_mcp_server(None, server, &secrets))
            }
            HubCall::ConnectConvex { key, repos } => {
                let http = crate::convex::client::Reqwest;
                done(self.connectors.connect_convex(&http, &key, repos).await)
            }
            HubCall::SetConvexRepos { id, repos } => done(self.connectors.set_convex_repos(&id, repos)),
            HubCall::DisconnectConvex { id } => done(self.connectors.disconnect_convex(&id)),
            HubCall::ConvexTables { id } => {
                let http = crate::convex::client::Reqwest;
                done(self.connectors.convex_tables(&http, &id).await)
            }
            HubCall::Convex { tab, repo, ask } => {
                let http = crate::convex::client::Reqwest;
                let listed = self.machine.listed_id();
                let machine = self.machine;
                let (here, named) = (listed.clone(), self.place_as.to_owned());
                let find = move |on: &str, id: &str| {
                    if !on.is_empty() && on != "local" && on != named && on != here {
                        return None;
                    }
                    machine.tabs().into_iter().find(|view| view.id == id)
                };
                done(self.connectors.convex(&http, &listed, &tab, repo.as_deref(), &find, ask).await)
            }
            HubCall::ConnectUpstash { .. }
            | HubCall::DisconnectUpstash
            | HubCall::SetBoxAccess { .. }
            | HubCall::AnswerBox { .. }
            | HubCall::OpenBox { .. }
            | HubCall::BoxStatus { .. }
            | HubCall::ExtendBox { .. }
            | HubCall::SetBoxClaudeToken { .. }
            | HubCall::RemoveBoxClaudeToken { .. }
            | HubCall::MintBoxClaudeToken { .. }
            | HubCall::CancelBoxClaudeToken { .. }
            | HubCall::BoxUsage
            | HubCall::StuckBoxes => Answer::Done(Err(NEEDS_THE_HUB_OFF.into())),
            HubCall::GenerateWorkflow { .. } => Answer::Done(Err("Generate with AI was removed: ask an agent to create the workflow".into())),
            HubCall::Team { op: crate::teams::TeamOp::AskPeople { .. }, .. } => Answer::Done(Err(NEEDS_THE_HUB.into())),
            HubCall::DelegationStats { days } if self.account.is_none() => match crate::delegate::stats_days(Some(u64::from(days))) {
                Ok(days) => done(Ok(self.machine.local_delegation_stats(days))),
                Err(error) => Answer::Done(Err(error)),
            },
            HubCall::Mirror { .. }
            | HubCall::Handback { .. }
            | HubCall::SetOfflineRunner { .. }
            | HubCall::ReleaseWorkflows
            | HubCall::Machines
            | HubCall::MachineStates
            | HubCall::Place { .. }
            | HubCall::RenameMachine { .. }
            | HubCall::RemoveMachine { .. }
            | HubCall::PairCode
            | HubCall::CreateInvite
            | HubCall::Invites
            | HubCall::RevokeInvite { .. }
            | HubCall::SharedProjects
            | HubCall::ShareProject { .. }
            | HubCall::SyncList { .. }
            | HubCall::SyncChanges { .. }
            | HubCall::SyncPush { .. }
            | HubCall::SyncPull { .. }
            | HubCall::SyncMarkExecutable { .. }
            | HubCall::ListSchedules
            | HubCall::SaveSchedule { .. }
            | HubCall::PauseSchedule { .. }
            | HubCall::DeleteSchedule { .. }
            | HubCall::ScheduleRuns { .. }
            | HubCall::SetReleaseProtection { .. }
            | HubCall::Import { .. }
            | HubCall::ExternalAgents
            | HubCall::AgentRequest { .. }
            | HubCall::ApproveAgent { .. }
            | HubCall::DenyAgent { .. }
            | HubCall::RevokeAgent { .. }
            | HubCall::Relay { .. }
            | HubCall::Overview
            | HubCall::ReadCaptain { .. }
            | HubCall::RepoLook { .. }
            | HubCall::ReadSession { .. }
            | HubCall::Delegate { .. }
            | HubCall::RemoteTask { .. }
            | HubCall::RemoteEnded { .. }
            | HubCall::RemoteAsked { .. }
            | HubCall::RecordTasks { .. }
            | HubCall::DelegationStats { .. }
            | HubCall::UnclaimedEnds
            | HubCall::SessionMarks
            | HubCall::MarkSessions { .. }
            | HubCall::ArchiveSession { .. }
            | HubCall::AutoArchive { .. }
            | HubCall::UnarchiveSession { .. }
            | HubCall::DeleteSession { .. }
            | HubCall::Act { .. }
            | HubCall::Confirmations
            | HubCall::DecideConfirmation { .. }
            | HubCall::Unlock { .. }
            | HubCall::AgentAudit { .. }
            | HubCall::ListIdeas { .. }
            | HubCall::SaveIdea { .. }
            | HubCall::DeleteIdea { .. }
            | HubCall::Reports { .. }
            | HubCall::ReportIssue { .. }
            | HubCall::Team { .. } => Answer::HubOnly,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Subject;
    use crate::connectors::FileStorage;
    use crate::graph::runner::{self, Session, Status};
    use crate::graph::{Graph, Node, NodeId, NodeKind, Watch};
    use crate::machine::refuse_real_home;
    use crate::teams::Situation;
    use std::collections::BTreeMap;

    fn live_run(dir: &std::path::Path, connector: &str, subject: Subject) {
        let graph = Graph {
            nodes: vec![Node {
                id: NodeId("t".into()),
                name: "start".into(),
                position: [0, 0],
                group_name: None,
                kind: NodeKind::Trigger {
                    watch: Watch::Manual,
                },
            }],
            edges: vec![],
        };
        let library = BTreeMap::from([(connector.to_owned(), graph)]);
        let mut run = runner::start(
            &library,
            connector,
            subject,
            serde_json::json!({}),
            "2026-11-19T00:00:00Z",
        );
        assert_eq!(run.status, Status::Running);
        run.session = Session::Live {
            machine: "local".into(),
            tab: "tab".into(),
            claude: None,
            since: 0,
            saw_working: false,
            await_working: false,
        };
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("runs.json"),
            serde_json::to_vec(&vec![run]).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn signed_out_run_set_and_ask_people_are_answered() {
        refuse_real_home();
        let dir = std::env::temp_dir().join(format!("pinned-signed-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let subject = Subject::LinearIssue {
            issue: "ISS-1".into(),
        };
        live_run(&dir, "wf", subject.clone());
        let connectors = Connectors::new(FileStorage::new(dir.clone()), "/bin/pinned".into());

        let set = signed_out(
            &connectors,
            Message::RunSet {
                connector: "wf".into(),
                subject: subject.clone(),
                name: "title".into(),
                value: "Export".into(),
            },
        );
        assert_eq!(
            set,
            SignedOut::Done(Ok("set {{vars.title}} for this workflow run".into()))
        );
        let stored = connectors
            .run_set("wf", &subject, "title", "Export")
            .unwrap();
        assert_eq!(stored, "set {{vars.title}} for this workflow run");

        let missing = signed_out(
            &connectors,
            Message::RunSet {
                connector: "wf".into(),
                subject: Subject::LinearIssue {
                    issue: "other".into(),
                },
                name: "title".into(),
                value: "x".into(),
            },
        );
        assert!(matches!(missing, SignedOut::Done(Err(_))));

        let ask = signed_out(
            &connectors,
            Message::AskPeople {
                situation: Situation::InDoubt,
                question: "Which?".into(),
                choices: vec!["red".into()],
                step: None,
            },
        );
        assert_eq!(ask, SignedOut::Done(Err(NEEDS_THE_HUB.into())));

        let _ = std::fs::remove_dir_all(dir);
    }

    struct Kept(std::sync::Mutex<Vec<crate::account_copy::RunSet>>);

    impl AccountSide for Kept {
        fn keep_run_set(&self, set: crate::account_copy::RunSet) {
            self.0.lock().unwrap().push(set);
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-local-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn machine_in(dir: &std::path::Path) -> Arc<crate::machine::Machine> {
        let launch = crate::machine::Launch {
            exe: dir.join("pinned"),
            bins: BTreeMap::new(),
            path_env: None,
            settings: dir.join("claude-settings.json"),
            mcp_config: None,
            reminders: false,
            plugin_dir: dir.join("plugin"),
            tutorial_helper: dir.join("helper"),
            sock: dir.join("sock"),
            vector_search: false,
            first_run_wait: std::time::Duration::from_millis(1),
            starting_limit: std::time::Duration::from_secs(1),
            claude_home: Some(dir.join("claude")),
        };
        crate::machine::Machine::new(launch, Some(dir.join("machine")), None, |_| {})
    }

    fn drive(hub: &LocalHub<'_>, call: HubCall) -> Answer {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(hub.answer(call))
    }

    #[test]
    fn the_copy_answers_settings_tastes_and_its_own_run_and_leaves_the_hub_the_rest() {
        refuse_real_home();
        let dir = scratch("answer");
        let connectors = Arc::new(Connectors::new(FileStorage::new(dir.join("store")), "/bin/pinned".into()));
        let machine = machine_in(&dir);
        let taste = dir.join("taste");
        let kept = Kept(std::sync::Mutex::new(vec![]));
        let signed_out = LocalHub { connectors: &connectors, machine: &machine, place_as: "local", taste_dir: Some(&taste), account: None };
        let settings = serde_json::to_value(connectors.get_settings()).unwrap();
        assert_eq!(drive(&signed_out, HubCall::GetSettings), Answer::Done(Ok(settings)));
        assert!(matches!(drive(&signed_out, HubCall::DelegationStats { days: 7 }), Answer::Done(Ok(_))));
        assert_eq!(drive(&signed_out, HubCall::ListMcpServers), Answer::Done(Err(SIGN_IN_MCP.into())));
        assert_eq!(drive(&signed_out, HubCall::LoadMcpServers), Answer::Done(Err(SIGN_IN_MCP.into())));
        assert_eq!(drive(&signed_out, HubCall::Machines), Answer::HubOnly);
        assert_eq!(drive(&signed_out, HubCall::SyncChanges { project: "github.com/acme/app".into(), since: None }), Answer::HubOnly);
        assert_eq!(drive(&signed_out, HubCall::Mirror { have: String::new(), login: "token".into() }), Answer::HubOnly);
        assert_eq!(drive(&signed_out, HubCall::DisconnectUpstash), Answer::Done(Err(NEEDS_THE_HUB_OFF.into())));
        let ask = HubCall::Team {
            team: None,
            op: crate::teams::TeamOp::AskPeople { situation: Situation::InDoubt, question: "Which?".into(), choices: vec!["red".into()], step: None },
        };
        assert_eq!(drive(&signed_out, ask), Answer::Done(Err(NEEDS_THE_HUB.into())));

        let scope = crate::taste::TasteScope::Captains;
        let patch = crate::taste::TastePatch { parallel: Some(2), ..crate::taste::TastePatch::default() };
        let set = drive(&signed_out, HubCall::SetTaste { scope: scope.clone(), patch });
        let Answer::Done(Ok(value)) = set else { panic!("set taste: {set:?}") };
        let tastes: crate::taste::Tastes = serde_json::from_value(value).unwrap();
        assert_eq!(tastes.captains.parallel, 2);
        let read = drive(&signed_out, HubCall::GetTastes);
        assert_eq!(read, Answer::Done(Ok(serde_json::to_value(&tastes).unwrap())));

        let subject = Subject::LinearIssue { issue: "ISS-9".into() };
        live_run(&dir.join("store"), "wf", subject.clone());
        let connectors = Arc::new(Connectors::new(FileStorage::new(dir.join("store")), "/bin/pinned".into()));
        let signed_in = LocalHub { connectors: &connectors, machine: &machine, place_as: "macbook", taste_dir: Some(&taste), account: Some(&kept) };
        let own = drive(
            &signed_in,
            HubCall::RunSet { connector: "wf".into(), subject: subject.clone(), name: "title".into(), value: "Export".into() },
        );
        assert_eq!(own, Answer::Done(Ok(serde_json::json!("set {{vars.title}} for this workflow run"))));
        assert!(kept.0.lock().unwrap().is_empty());
        let other = Subject::LinearIssue { issue: "other".into() };
        let held = drive(
            &signed_in,
            HubCall::RunSet { connector: "wf".into(), subject: other.clone(), name: "title".into(), value: "Later".into() },
        );
        assert_eq!(held, Answer::Done(Ok(serde_json::json!("kept {{vars.title}} until the hub takes this run"))));
        let saved = kept.0.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].connector, "wf");
        assert_eq!(saved[0].subject, other);
        assert_eq!(saved[0].value, "Later");
        assert_eq!(drive(&signed_in, HubCall::ListMcpServers), Answer::Done(Ok(serde_json::json!([]))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_box_workflow_is_refused_without_the_hub() {
        refuse_real_home();
        let dir = scratch("box");
        let connectors = Arc::new(Connectors::new(FileStorage::new(dir.join("store")), "/bin/pinned".into()));
        let workflow = serde_json::json!({
            "name": "Nightly",
            "enabled": true,
            "trigger": {"kind": "manual"},
            "folder": "/work",
            "model": "",
            "effort": "medium",
            "permissionMode": "acceptEdits",
            "worktree": false,
            "instructions": "Check the build.",
            "machine": {"kind": "upstash-box"},
            "graph": {
                "nodes": [
                    {"id": "t", "name": "Trigger", "position": [0, 0], "type": "trigger", "watch": {"kind": "manual"}},
                    {"id": "s", "name": "Do", "position": [1, 0], "type": "session"}
                ],
                "edges": [{"from": "t", "port": "out", "to": "s"}]
            }
        });
        let saved = connectors.save_connector(serde_json::from_value(workflow).unwrap()).unwrap();
        let machine = machine_in(&dir);
        let signed_out = LocalHub { connectors: &connectors, machine: &machine, place_as: "local", taste_dir: None, account: None };
        let refused = drive(&signed_out, HubCall::RunWorkflow { id: saved.id.clone() });
        assert_eq!(refused, Answer::Done(Err(BOX_SIGNED_OUT.into())));
        let signed_in_place = LocalHub { connectors: &connectors, machine: &machine, place_as: "macbook", taste_dir: None, account: Some(&Kept(std::sync::Mutex::new(vec![]))) };
        let refused = drive(&signed_in_place, HubCall::RunWorkflow { id: saved.id });
        assert_eq!(refused, Answer::Done(Err(BOX_OFF.into())));
        let _ = std::fs::remove_dir_all(dir);
    }
}
