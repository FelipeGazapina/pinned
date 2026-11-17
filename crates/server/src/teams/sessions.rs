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
