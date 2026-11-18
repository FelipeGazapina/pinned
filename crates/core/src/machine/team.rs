//! `Machine::team_tool`: a team tool's call (`team_tools::TeamTool`), handed to the module that
//! owns the tool. Units fill in their own module, not this one.

use super::Machine;
use crate::team_tools::TeamTool;

impl Machine {
    /// Answers a team tool called from tab `tab` or delegated task `task`.
    pub(super) fn team_tool(&self, tab: Option<&str>, task: Option<&str>, tool: TeamTool) -> Result<String, String> {
        match tool {
            TeamTool::TaskFix(call) => self.task_fix(tab, task, call),
            TeamTool::FeatureMapRead(call) => self.feature_map_read(tab, task, call),
            TeamTool::FeatureMapSet(call) => self.feature_map_set(tab, task, call),
            TeamTool::ProfilePropose(call) => self.profile_propose(tab, task, call),
            TeamTool::BoardNote(call) => self.board_note(tab, task, call),
            TeamTool::BoardRead(call) => self.board_read(tab, task, call),
            TeamTool::AskWorker(call) => self.ask_worker(tab, task, call),
        }
    }
}

/// What a team tool answers until its unit turns it on.
pub(super) fn not_yet(name: &str) -> Result<String, String> {
    Err(format!("{name} is not available in this build"))
}

#[cfg(test)]
mod tests {
    use super::super::tests::launch;
    use super::*;
    use serde_json::json;

    #[test]
    fn delegate_task_with_a_profile_or_a_chain_is_refused_and_starts_nothing() {
        let m = Machine::new(launch(), None, None, |_| {});
        for (args, error) in [
            (json!({ "task": "Fix it.", "profile": "scout" }), "profile is not available in this build"),
            (json!({ "task": "Fix it.", "then": ["tester"] }), "then is not available in this build"),
        ] {
            assert_eq!(crate::delegate::parse_request(&args), Err(error.into()), "the MCP side refuses it");
            let mut request = serde_json::to_value(crate::delegate::parse_request(&json!({ "task": "Fix it." })).unwrap()).unwrap();
            request.as_object_mut().unwrap().extend(args.as_object().unwrap().iter().filter(|(k, _)| *k != "task").map(|(k, v)| (k.clone(), v.clone())));
            let request: crate::delegate::Request = serde_json::from_value(request).unwrap();
            assert_eq!(m.start_delegate(Some("tab-1".into()), None, None, request.clone()), Err(error.into()), "the machine refuses one that reached it");
            let origin = crate::delegate::Origin {
                machine: "m-mac".into(),
                machine_name: "mac".into(),
                tab: "tab-7".into(),
                task: "task-mac".into(),
                access: crate::delegate::Access::Auto,
                delegator: Default::default(),
                limits: Default::default(),
            };
            assert_eq!(m.start_for_origin(origin, request, None, None), Err(error.into()), "so does one another machine sent");
        }
        assert!(m.delegates.records().is_empty() && m.delegates.working_for("tab:tab-1") == 0, "a refused call started a task");
        assert_eq!(crate::delegate::parse_request(&json!({ "task": "Fix it.", "then": "tester" })), Err("then must be a list of profile names".into()));
        let plain = crate::delegate::parse_request(&json!({ "task": "Fix it.", "profile": null, "then": [] })).unwrap();
        assert_eq!((plain.profile, plain.then), (None, vec![]));
    }

    #[test]
    fn every_team_tool_answers_that_it_is_not_available_yet() {
        let m = Machine::new(launch(), None, None, |_| {});
        for name in crate::team_tools::NAMES {
            let args = match name {
                "task_fix" | "board_read" => json!({ "taskId": "t1" }),
                "feature_map_set" => json!({ "feature": "login", "files": [] }),
                "profile_propose" => json!({ "profile": { "name": "scout" } }),
                "board_note" => json!({ "file": "a.rs", "text": "x" }),
                "ask_worker" => json!({ "taskId": "t1", "question": "x" }),
                _ => json!({}),
            };
            let tool = crate::team_tools::request(name, &args).unwrap();
            assert_eq!(m.team_tool(Some("tab-1"), None, tool), Err(format!("{name} is not available in this build")));
        }
    }
}
