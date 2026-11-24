//! The machine side of worker profiles: U5's signed-out store and `profile_propose`, and U6's
//! `delegate_task` with `profile`.

use super::Machine;
use crate::delegate::{token_ok, Caller, Interaction, Origin, Request};
use crate::history;
use crate::profiles::{ProfileAccess, WorkerProfile};
use crate::taste::{self, same_name, TasteScope, Tastes};
use crate::team_tools::ProfilePropose;

impl Machine {
    /// `Tastes::set_worker_profiles` on this machine's own tastes, for a signed-out app.
    pub fn set_local_worker_profiles(&self, project: Option<&str>, profiles: Vec<WorkerProfile>) -> Result<Tastes, String> {
        taste::change(self.orchestration_dir.as_deref(), |tastes| tastes.set_worker_profiles(project, profiles))
    }

    /// `delegate_task`'s request as the profile it names sets it up. The name is looked up in the
    /// caller's project's profiles, then the account's. The profile fills in the model, effort and
    /// tools the call leaves out, makes the worker read-only when it is, and puts its prompt ahead
    /// of the task. A replayed request, and one another machine sent already set up, pass as they are.
    pub(super) fn profile_request(&self, caller: &Caller, mut request: Request) -> Result<Request, String> {
        let Some(asked) = request.profile.clone() else { return Ok(request) };
        let sent = caller.tab.as_deref().is_some_and(|tab| Origin::of_key(tab).is_some());
        if sent || self.delegates.requested(&caller.key(), request.client_request_id.as_deref()).is_some() {
            return Ok(request);
        }
        let cwd = match (&caller.tab, &caller.task) {
            (Some(tab), _) => self.tab(tab, |t| t.cwd.clone())?,
            (None, task) => {
                let id = task.as_deref().unwrap_or_default();
                self.delegates.lineage(id).ok_or_else(|| format!("no task {id}"))?.cwd
            }
        };
        let tastes = self.tastes();
        let lists = &tastes.worker_profiles;
        let own = match super::captain_scope(&history::project_of(&cwd)) {
            TasteScope::Project { repo } => lists.projects.get(&repo).cloned().unwrap_or_default(),
            _ => vec![],
        };
        let Some(profile) = own.iter().chain(&lists.account).find(|profile| same_name(&profile.name, &asked)) else {
            let mut names: Vec<&str> = vec![];
            for profile in own.iter().chain(&lists.account) {
                if !names.iter().any(|name| same_name(name, &profile.name)) {
                    names.push(profile.name.trim());
                }
            }
            return Err(match names.is_empty() {
                true => format!("no worker profile is named {asked:?}, and there are none yet: the user adds them under Settings › Way of working. Nothing started."),
                false => format!("no worker profile is named {asked:?}. The profiles here are {}. Nothing started.", names.join(", ")),
            });
        };
        let name = profile.name.trim();
        // A model or effort the call names wins. One from the profile passes the call's own checks.
        if request.target.model.is_none() {
            request.target.model = profile.model.as_ref().map(|model| model.model.clone());
            if let Some(model) = request.target.model.as_deref().filter(|model| !token_ok(model)) {
                return Err(format!("{name}'s model {model:?} is not a model id"));
            }
        }
        if request.target.effort.is_none() {
            request.target.effort.clone_from(&profile.effort);
            if let Some(effort) = request.target.effort.as_deref().filter(|effort| !token_ok(effort)) {
                return Err(format!("{name}'s effort {effort:?} is not an effort"));
            }
        }
        if profile.access == ProfileAccess::ReadOnly {
            request.interaction = Interaction::Plan;
        }
        if !profile.prompt.trim().is_empty() {
            request.task = format!("{}\n\n{}", profile.prompt.trim(), request.task);
        }
        if request.tools.is_empty() {
            request.tools.clone_from(&profile.tools);
        }
        request.profile = Some(profile.name.clone());
        Ok(request)
    }

    pub(super) fn profile_propose(&self, _tab: Option<&str>, _task: Option<&str>, _call: ProfilePropose) -> Result<String, String> {
        super::team::not_yet("profile_propose")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::tests::{delegate_task, git, limited_captain, placing_captain, placing_hub, started};
    use super::*;
    use crate::delegate::{parse_request, Access};
    use crate::taste::TastePatch;
    use crate::wire::HubCall;
    use serde_json::{json, Value};

    fn profiles(list: Value) -> Vec<WorkerProfile> {
        serde_json::from_value(list).unwrap()
    }

    fn status(m: &Machine, captain: &str, reply: &Value) -> Value {
        serde_json::from_str(&m.task_status(Some(captain.to_owned()), None, reply["taskId"].as_str().unwrap()).unwrap()).unwrap()
    }

    fn launched(dir: &std::path::Path, reply: &Value) -> Vec<String> {
        started(&dir.join("claude"), reply["tabId"].as_str().unwrap())
    }

    fn has(args: &[String], pair: [&str; 2]) -> bool {
        args.windows(2).any(|w| w == pair)
    }

    /// Gives the Captain's folder `app` the origin `github.com/acme/app`.
    fn in_acme_app(dir: &std::path::Path) {
        git(&dir.join("app"), &["init", "-q"]);
        git(&dir.join("app"), &["remote", "add", "origin", "https://github.com/acme/app.git"]);
    }

    #[test]
    fn a_captains_delegate_task_with_a_profile_runs_the_worker_as_the_profile_sets_it_up() {
        let (m, dir, captain) = limited_captain("profile-account", TastePatch::default());
        let scout = json!({ "name": "Scout", "model": { "model": "claude-haiku-5-5" }, "effort": "high", "prompt": "You map code.\nYou never edit it.", "tools": ["Read", "Grep"] });
        m.set_local_worker_profiles(None, profiles(json!([scout]))).unwrap();
        let reply = delegate_task(&m, &captain, json!({ "task": "Map the parser.", "title": "Parser", "profile": "  scout ", "target": { "providerInstanceId": "work" } }));
        let args = launched(&dir, &reply);
        assert!(has(&args, ["--model", "claude-haiku-5-5"]) && has(&args, ["--effort", "high"]), "{args:?}");
        assert!(args.iter().any(|arg| arg == "Parser\n\nGranted tools: Read, Grep.\n\nYou map code.\nYou never edit it.\n\nMap the parser."), "{args:?}");
        assert_eq!(status(&m, &captain, &reply)["profile"], "Scout", "the name as the profile stores it");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_model_and_effort_the_call_names_win_over_its_profiles() {
        let (m, dir, captain) = limited_captain("profile-call-wins", TastePatch::default());
        m.set_local_worker_profiles(None, profiles(json!([{ "name": "scout", "model": { "model": "claude-haiku-5-5" }, "effort": "high", "tools": ["Read"] }]))).unwrap();
        let call = json!({ "task": "Map the parser.", "title": "Parser", "profile": "scout", "tools": ["Bash"], "target": { "providerInstanceId": "work", "model": "claude-sonnet-5-5", "options": { "effort": "low" } } });
        let reply = delegate_task(&m, &captain, call);
        let args = launched(&dir, &reply);
        assert!(has(&args, ["--model", "claude-sonnet-5-5"]) && has(&args, ["--effort", "low"]), "{args:?}");
        assert!(args.iter().any(|arg| arg == "Parser\n\nGranted tools: Bash.\n\nMap the parser."), "the call's tools too: {args:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_project_profile_wins_over_the_account_profile_of_the_same_name() {
        let (m, dir, captain) = limited_captain("profile-project", TastePatch::default());
        in_acme_app(&dir);
        m.set_local_worker_profiles(None, profiles(json!([{ "name": "scout", "model": { "model": "claude-haiku-5-5" } }]))).unwrap();
        m.set_local_worker_profiles(Some("github.com/acme/app"), profiles(json!([{ "name": "Scout", "model": { "model": "claude-sonnet-5-5" } }]))).unwrap();
        let reply = delegate_task(&m, &captain, json!({ "task": "Map the parser.", "profile": "SCOUT", "target": { "providerInstanceId": "work" } }));
        let args = launched(&dir, &reply);
        assert!(has(&args, ["--model", "claude-sonnet-5-5"]), "{args:?}");
        assert_eq!(status(&m, &captain, &reply)["profile"], "Scout");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unknown_or_only_proposed_profile_is_refused_with_the_names_and_starts_nothing() {
        let (m, dir, captain) = limited_captain("profile-unknown", TastePatch::default());
        let refused = |name: &str| m.start_delegate(Some(captain.clone()), None, None, parse_request(&json!({ "task": "Map the parser.", "profile": name })).unwrap()).unwrap_err();
        assert_eq!(refused("scout"), "no worker profile is named \"scout\", and there are none yet: the user adds them under Settings › Way of working. Nothing started.");
        in_acme_app(&dir);
        m.set_local_worker_profiles(Some("github.com/acme/app"), profiles(json!([{ "name": "fixer" }]))).unwrap();
        m.set_local_worker_profiles(None, profiles(json!([{ "name": "scout" }, { "name": "Fixer" }]))).unwrap();
        m.propose_worker_profile(&captain, profiles(json!([{ "name": "tester" }])).remove(0)).unwrap();
        let names = "The profiles here are fixer, scout. Nothing started.";
        assert_eq!(refused("ghost"), format!("no worker profile is named \"ghost\". {names}"));
        assert_eq!(refused("tester"), format!("no worker profile is named \"tester\". {names}"), "a proposal is not the user's yet");
        assert!(m.delegates.records().is_empty() && m.delegates.working_for(&format!("tab:{captain}")) == 0 && m.state().workers == 0, "a refused call started a task");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_read_only_profile_makes_a_read_only_worker_and_an_edit_profile_never_widens_access() {
        let (m, dir, captain) = limited_captain("profile-read-only", TastePatch::default());
        m.set_local_worker_profiles(None, profiles(json!([{ "name": "reader", "access": "read_only" }, { "name": "editor", "access": "edit" }]))).unwrap();
        m.tab(&captain, |t| t.bypass = true).unwrap();
        assert_eq!(m.tab(&captain, |t| t.child_access()).unwrap(), Access::Full);
        let reply = delegate_task(&m, &captain, json!({ "task": "Read the parser.", "profile": "reader", "runtimeMode": "full-access", "target": { "providerInstanceId": "work" } }));
        assert_eq!(m.delegates.get(reply["taskId"].as_str().unwrap()).unwrap().access, Access::ReadOnly);
        let read_only = crate::delegate::claude_read_only_args();
        let args = launched(&dir, &reply);
        assert!(args.windows(read_only.len()).any(|flags| flags == read_only), "{args:?}");
        m.tab(&captain, |t| t.permission_mode = Some("plan".into())).unwrap();
        let reply = delegate_task(&m, &captain, json!({ "task": "Fix the parser.", "profile": "editor", "target": { "providerInstanceId": "work" } }));
        assert_eq!(m.delegates.get(reply["taskId"].as_str().unwrap()).unwrap().access, Access::ReadOnly, "an edit profile under a read-only Captain");
        m.quit();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_profile_worker_sent_to_another_machine_carries_what_the_profile_set_and_not_its_name() {
        let (m, dir, captain) = placing_captain("profile-remote");
        let calls = placing_hub(&m, vec![], &[]);
        let reader = json!({ "name": "Reader", "access": "read_only", "model": { "model": "claude-sonnet-5-5" }, "effort": "high", "prompt": "You read.", "tools": ["Read"] });
        m.set_local_worker_profiles(None, profiles(json!([reader]))).unwrap();
        let reply = delegate_task(&m, &captain, json!({ "task": "Map the parser.", "profile": "reader", "target": { "machine": "m-mini", "providerInstanceId": "work" } }));
        let sent = calls.lock().unwrap().iter().find_map(|call| match call {
            HubCall::Delegate { request, .. } => Some(serde_json::to_value(request).unwrap()),
            _ => None,
        });
        let sent = sent.expect("the worker went to the hub");
        assert_eq!(
            (&sent["target"]["model"], &sent["target"]["effort"], &sent["interaction"], &sent["task"], &sent["tools"], sent.get("profile")),
            (&json!("claude-sonnet-5-5"), &json!("high"), &json!("plan"), &json!("You read.\n\nMap the parser."), &json!(["Read"]), None)
        );
        assert_eq!(status(&m, &captain, &reply)["profile"], "Reader", "the machine that asked keeps the name");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_worker_another_machine_sent_with_a_profile_runs_as_sent_and_keeps_the_name() {
        let (m, dir, _captain) = limited_captain("profile-origin", TastePatch::default());
        in_acme_app(&dir);
        let origin = Origin {
            machine: "m-mac".into(),
            machine_name: "mac".into(),
            tab: "tab-7".into(),
            task: "task-mac".into(),
            access: Access::Auto,
            delegator: Default::default(),
            limits: Default::default(),
        };
        let request = parse_request(&json!({ "task": "You read.\n\nMap the parser.", "title": "Parser", "profile": "reader", "target": { "providerInstanceId": "work" } })).unwrap();
        let reply: Value = serde_json::from_str(&m.start_for_origin(origin, request, Some("https://github.com/acme/app.git".into()), None).unwrap()).unwrap();
        let args = launched(&dir, &reply);
        assert!(args.iter().any(|arg| arg == "Parser\n\nYou read.\n\nMap the parser."), "no profile here, and no second prompt: {args:?}");
        assert_eq!(m.delegates.get(reply["taskId"].as_str().unwrap()).unwrap().profile.as_deref(), Some("reader"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_replayed_delegate_task_with_a_profile_answers_its_task_after_the_profile_is_gone() {
        let (m, dir, captain) = limited_captain("profile-replay", TastePatch::default());
        m.set_local_worker_profiles(None, profiles(json!([{ "name": "Scout", "prompt": "You map code." }]))).unwrap();
        let call = json!({ "task": "Map the parser.", "profile": "scout", "clientRequestId": "req-1", "target": { "providerInstanceId": "work" } });
        let first = delegate_task(&m, &captain, call.clone());
        let _ = launched(&dir, &first);
        m.set_local_worker_profiles(None, vec![]).unwrap();
        let again = delegate_task(&m, &captain, call);
        assert_eq!((&again["taskId"], &again["profile"]), (&first["taskId"], &json!("Scout")));
        assert_eq!(m.delegates.working_for(&format!("tab:{captain}")), 1, "the replay started nothing");
        let _ = std::fs::remove_dir_all(dir);
    }
}
