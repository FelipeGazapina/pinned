//! What an agent may do as the user: one entry per `Call` and `HubCall`, which the `actions` and
//! `act` tools offer to every session pinned launches. The hub runs each act as the account that
//! owns the session's machine, writes it to the audit trail, and turns a `confirm` action into a
//! confirmation the user decides in the app. `docs/global-orchestrator/CONTRACTS.md` section 13.

use crate::wire::{Call, HubCall, TabId, TabView};
use schemars::generate::SchemaSettings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Which enum an action's call is: a `Call` a machine answers, or a `HubCall` the hub answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Node,
    Hub,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Runs and answers.
    Read,
    /// Runs and answers.
    Write,
    /// Runs only once the user approves it in the app.
    Confirm,
    /// Never runs for an agent.
    Excluded,
}

#[derive(Clone, Copy, Debug)]
pub struct Action {
    /// What `act` takes: the wire `cmd`, or `node_<cmd>` for a `Call` that shares its `cmd` with a `HubCall`.
    pub name: &'static str,
    pub side: Side,
    pub cmd: &'static str,
    pub class: Class,
    /// `act` runs it. An unwired action is classified and refused until a unit wires it.
    pub wired: bool,
    /// It starts a session or other work, so it runs only through the delegation limits.
    pub starts_work: bool,
    /// What agents read in `actions`; for an excluded action, why it is closed.
    pub about: &'static str,
    /// Fields of its answer an agent never gets, at any depth.
    pub hides: &'static [&'static str],
    /// Dotted paths of arguments an agent may send though their names read as a key, such as a
    /// project's `project.key`, which names a repository.
    pub sends: &'static [&'static str],
    /// Refuses arguments that parse but an agent may not send. Its error is the refusal.
    pub check: Option<fn(&Parsed) -> Result<(), String>>,
    /// A `Call` that runs only on another machine of the account: on its own machine a session
    /// has its own shell and files.
    pub elsewhere: bool,
    /// The fields the user types in the app when approving a credential action. An agent leaves
    /// them out, and `with_entered` adds them to the stored arguments at the decision.
    pub asks: &'static [&'static str],
}

impl Action {
    const fn wired(self) -> Action {
        Action { wired: true, ..self }
    }

    const fn named(self, name: &'static str) -> Action {
        Action { name, ..self }
    }

    const fn starts_work(self) -> Action {
        Action { starts_work: true, ..self }
    }

    const fn hides(self, hides: &'static [&'static str]) -> Action {
        Action { hides, ..self }
    }

    const fn sends(self, sends: &'static [&'static str]) -> Action {
        Action { sends, ..self }
    }

    const fn elsewhere(self) -> Action {
        Action { elsewhere: true, ..self }
    }

    const fn asks(self, asks: &'static [&'static str]) -> Action {
        Action { asks, ..self }
    }

    /// Adds the action's argument check, which `parse` runs on the call it built.
    pub const fn checks(self, check: fn(&Parsed) -> Result<(), String>) -> Action {
        Action { check: Some(check), ..self }
    }

    /// `machine` for a `Call`, which `act` runs on the session's machine or the one it names.
    pub fn on(&self) -> &'static str {
        match self.side {
            Side::Node => "machine",
            Side::Hub => "hub",
        }
    }
}

const fn n(cmd: &'static str, class: Class, about: &'static str) -> Action {
    Action { name: cmd, side: Side::Node, cmd, class, wired: false, starts_work: false, about, hides: &[], sends: &[], check: None, elsewhere: false, asks: &[] }
}

const fn h(cmd: &'static str, class: Class, about: &'static str) -> Action {
    Action { name: cmd, side: Side::Hub, cmd, class, wired: false, starts_work: false, about, hides: &[], sends: &[], check: None, elsewhere: false, asks: &[] }
}

use Class::{Confirm, Excluded, Read, Write};

const GIT: &str = "git and GitHub: run git and gh in your own shell";
const FILES: &str = "files of a session's folder: use your own shell and files";
const TERMINAL: &str = "the app's terminal plumbing";
const SYNC: &str = "machines keep their projects in step with the hub by themselves";

/// Every `Call` and `HubCall`, as agents see it. `every_wire_call_has_one_action` fails when a
/// variant has no entry here.
pub const ACTIONS: &[Action] = &[
    // Sessions on a machine.
    n("tabs", Read, "Every session on the machine as JSON: id, folder, agent account, title, status, role, Claude session id and who started it.").wired(),
    n("open_tab", Write, "Opens a new session tab in the folder path (null: the machine's default folder), not started, and answers its id. Start it with start_claude. bypass runs it with permissions skipped.").wired(),
    n("open_scratch", Write, "Opens a new session tab in the folder for threads without a project, not started, and answers its id.").wired(),
    n("auto_resume", Excluded, "a thread's own switch for continuing after a usage limit, which the user sets"),
    n("start_claude", Write, "Starts the session in tab and answers its folder. It becomes your worker: it counts against its project's workers-at-once limit, is held to the time limit and budget, its first finished turn ends it as a task, and you get its notice. cols and rows size its terminal; 120 and 32 suit.").wired().starts_work(),
    n("set_bypass", Write, "Restarts a session with permissions skipped (on: true) or asked (on: false), resuming its conversation.").wired(),
    n("set_model", Write, "Restarts a session's conversation on model, a slug from the models list_accounts gives its Claude Code account, optionally moving it to agent account first. A model above the tier its project allows is refused.").wired(),
    n("restart_claude", Write, "Restarts a session on its conversation, so its skills, plugins and MCP servers load again.").wired(),
    n("write_input", Excluded, TERMINAL),
    n("resize_pty", Excluded, TERMINAL),
    n("wake", Excluded, TERMINAL),
    n("close_tab", Excluded, "closing a thread loses its place in the sidebar: agents archive it with archive_session, which the user can undo, or ask to delete it with delete_session"),
    n("stop_turn", Write, "Stops the turn a session is running, as the app's stop button does. The session stays open for the next prompt.").wired(),
    n("set_folder", Write, "Moves a tab that has not started to another folder.").wired(),
    n("list_projects", Read, "The machine's projects, most recent first: folder, name, repo and the agent account its sessions run under.").wired(),
    n("project_history", Read, "The saved turns of the project at path, newest first, only one Claude session's when session is given.").wired(),
    n("git_remote", Excluded, GIT),
    n("browse", Excluded, "the folder picker's typeahead: list folders in your own shell"),
    n("create_project", Write, "Makes a new git repository named after name in ~/pinned, with a first commit, and opens it as a project.").wired(),
    n("publish_project", Write, "Pushes the project at path to a new private GitHub repository and answers its URL.").wired(),
    n("clone", Excluded, "it takes a GitHub token: the hub's clone action adds the account's own").named("node_clone"),
    n("make_folder", Write, "Makes the folder path with its parents and answers it, canonical.").wired(),
    n("search_threads", Read, "The sessions on the machine whose conversations hold query, prompt matches first.").wired(),
    n("open_run", Excluded, "the hub's way to reopen an archived conversation's tab for an agent without starting it"),
    n("run", Excluded, "the hub's way to start a workflow's session: its raw arguments and environment could pass for a workflow's"),
    n("deliver", Excluded, "the hub's way into a workflow's session, matched by environment"),
    n("run_now", Excluded, "the hub's way to start a workflow's Run now on a machine under its limits: call run_workflow"),
    n("upload", Excluded, "the app's file upload: you have your own files"),
    n("read_upload", Excluded, "the app's picture previews"),
    n("read_image", Excluded, "the app's picture previews"),
    n("learn_model_menu", Excluded, "the app's reading of claude's /model menu off a tab's screen"),
    n("list_accounts", Read, "The agent accounts on the machine (Claude Code, Codex, Cursor, OpenCode Go), each with whether it is signed in.").wired(),
    n("add_account", Confirm, "Adds an agent account on the machine. The user enters any API key in the app.").wired().asks(&["key"]),
    n("set_account_key", Confirm, "Replaces an agent account's API key. The user enters it in the app.").wired().asks(&["key"]),
    n("remove_account", Confirm, "Removes an agent account from the machine; its projects fall back to the default account.").wired(),
    n("set_project_account", Write, "Runs the project at path under another agent account, or the default one with null.").wired(),
    n("open_orchestrator", Write, "Opens the Captain tab of the project at path, not started, and answers its id.").wired(),
    n("enqueue", Write, "Hands text to the Captain of the project at path: typed when it is idle, its first prompt when stopped, else queued. It is signed with your own role. Starting a stopped Captain is held to the project's tier, effort and workers-at-once limits.").wired().starts_work(),
    n("orchestrator_report", Excluded, "a Captain's report to the global orchestrator: a Captain calls tell_orchestrator"),
    n("open_global_orchestrator", Write, "Opens this machine's global orchestrator tab, not started, and answers its id. Only the global orchestrator's own session may call it.").wired(),
    n("open_taste_chat", Write, "Opens a thread that adjusts a scope's way of working by talking, not started, and answers its id."),
    n("global_orchestrator_settings", Read, "The machine's global orchestrator settings: its name, whether this machine hosts it, and its busy and report policies.").wired(),
    n("set_global_orchestrator_settings", Write, "Saves the machine's global orchestrator settings.").wired(),
    n("auto_archive_settings", Excluded, "the user's own setting for when finished threads archive themselves"),
    n("set_auto_archive_settings", Excluded, "the user's own setting for when finished threads archive themselves"),
    n("set_background", Excluded, "the user's own setting for whether the app on a machine keeps a window"),
    n("read_session", Read, "A session's state and its last items (20, at most 50) of conversation, read from the end of its transcript, on the session's machine or the one machine names.").named("node_read_session").wired(),
    n("memory_search", Read, "The memories of the project at path that best match query, best first, or the latest for an empty query. Any project of the account: pass machine for one on another machine.").wired(),
    n("memory_save", Write, "Saves a memory in the project at path, any project of the account (pass machine for another machine). With id, replaces that memory. Refused when the project has one with the same title or nearly the same meaning.").wired(),
    n("memory_forget", Write, "Removes the memory id of the project at path, any project of the account.").wired(),
    n("memory_status", Read, "How many memories the project at path has and whether search also matches by meaning.").wired(),
    n("sign_in", Confirm, "Opens the CLI's login for an agent account in a new tab, where the user signs in.").wired(),
    n("setup_token", Excluded, "the hub's way to have a machine mint a Claude token for boxes, which only the hub may receive: call mint_box_claude_token"),
    n("cancel_setup_token", Excluded, "the hub's way to stop a machine's claude setup-token: call cancel_box_claude_token"),
    n("switch_account", Write, "Moves a session's conversation to another agent account signed in on the machine and resumes it there.").wired(),
    n("repo_look", Excluded, "the global orchestrator's repo_overview, repo_read and repo_search, which the hub sends after its permission and rate checks").named("node_repo_look"),
    n("thread", Read, "A session's conversation items from byte from of its transcript (0: the start) and the offset to read next. node_read_session reads its end.").wired(),
    n("prompt", Write, "Types text into another running session as one prompt and submits it. A Captain takes enqueue instead, and the global orchestrator takes tell_orchestrator.").wired(),
    n("workspace", Excluded, GIT),
    n("pull_request", Excluded, GIT),
    n("pr_status", Excluded, GIT),
    n("favicon", Excluded, "the sidebar's project icons"),
    n("regenerate_title", Write, "Writes a new title for a session's conversation and answers it. An empty thread reads the session's own conversation.").wired(),
    n("pr_list", Excluded, GIT),
    n("pr_action", Excluded, GIT),
    n("diff", Read, "The diff of the folder of session tab on another machine of the account, against HEAD or the default branch (scope). Not on your own machine: there, run git in your own shell.").wired().elsewhere(),
    n("ports", Excluded, "the app's browser panel: list ports in your own shell"),
    n("editors", Excluded, "opens the user's editors on their screen"),
    n("open_in", Excluded, "opens the user's editors on their screen"),
    n("commands", Excluded, "the composer's slash-command list"),
    n("files", Read, "The files and folders of the folder of session tab on another machine of the account, relative. Not on your own machine: there, use your own shell and files.").wired().elsewhere(),
    n("skills", Read, "Every skill folder on the machine: personal, project and plugin skills, and where each copy lives.").wired(),
    n("read_skill", Read, "The text of file (SKILL.md when left out) in the skill folder dir that skills lists.").wired(),
    n("mcp_servers", Read, "The MCP servers Claude Code sessions on the machine load under agent account account (the project at path's account, else the default one): user and local scope from that account's .claude.json, project scope from path's .mcp.json. Each gives its name, scope, transport, and only its program or its URL's scheme and host, never its arguments, environment or headers. MCP servers that plugins bring are not listed.").wired(),
    n("mcp_importable", Excluded, "the app's import form, which reads each server's arguments and URL in full: mcp_servers lists them, and import_mcp_server asks to add one"),
    n("save_skill", Write, "Writes a skill file to each listed copy unchanged since its read, then reloads the sessions that load it.").wired(),
    n("branches", Excluded, GIT),
    n("checkout", Excluded, GIT),
    n("pull_requests", Excluded, GIT),
    n("pull_request_stack", Excluded, GIT),
    n("pull_request_filter_options", Excluded, GIT),
    n("pull_request_files", Excluded, GIT),
    n("token_usage", Read, "The tokens and cost the machine's Claude transcripts record from unix time since on, by model and day.").wired(),
    n("usage_prices", Read, "The model prices and mappings the user set on the machine.").wired(),
    n("set_usage_prices", Write, "Sets or clears model prices and mappings on the machine.").wired(),
    n("usage_limits", Read, "The account and plan limits Claude's own usage check reports.").wired(),
    n("page_meta", Excluded, "the app's link previews"),
    n("refs", Excluded, GIT),
    n("git", Excluded, GIT),
    n("project_scripts", Read, "The scripts of a session's project, which its header runs.").wired(),
    n("save_project_scripts", Write, "Replaces the scripts of a session's project.").wired(),
    n("checkout_pr", Excluded, GIT),
    n("set_workspace", Write, "Moves a session that is not in a turn to a new worktree of its project (worktree: \"new\"), an existing one, or back to the project's checkout (null), and restarts it there on its conversation.").wired(),
    n("read_file", Read, "The text of path in the folder of session tab on another machine of the account, or in its repository when root, as of git rev when given. Not on your own machine: there, use your own shell and files.").wired().elsewhere(),
    n("watch_pr", Write, "Starts or stops watching a pull request for a session, which wakes it when the checks finish, someone comments, or its branch conflicts."),
    n("file_url", Excluded, FILES),
    n("write_file", Excluded, FILES),
    n("search", Read, "The lines that hold query in the folder of session tab on another machine of the account. Not on your own machine: there, use your own shell.").wired().elsewhere(),
    n("pr_edit", Excluded, GIT),
    n("pr_comment", Excluded, GIT),
    n("pr_review", Excluded, GIT),
    n("pr_rerun", Excluded, GIT),
    n("pr_guide", Excluded, GIT),
    n("pr_change", Excluded, GIT),
    n("pr_candidates", Excluded, GIT),
    n("pr_stack", Excluded, GIT),
    n("open_shell", Excluded, TERMINAL),
    n("close_shell", Excluded, TERMINAL),
    n("busy_shells", Excluded, TERMINAL),
    n("device_hub", Write, "Starts the machine's device hub, which mirrors its simulators and emulators, and answers its address.").wired(),
    n("device_agents", Write, "Whether new sessions may drive the machine's simulators and emulators, after setting it to allow when given.").wired(),
    n("device", Write, "Runs op against the machine's simulators and emulators.").wired(),
    n("subagent", Read, "The conversation of the subagent a session's Agent tool call tool started.").wired(),
    n("rewind", Write, "Restarts a session on a copy of its conversation that stops before the prompt before, optionally putting its files back as they were.").wired(),
    n("fork", Write, "Opens a new tab, not started, that resumes a copy of a session's conversation through the message through.").wired(),
    n("save_file", Excluded, FILES),
    n("open_task", Write, "Opens a tab, not started, that resumes a delegated task's conversation."),
    n("cancel_task", Write, "Stops a delegated task that the session in tab owns."),
    n("deliver_task", Excluded, "the app's send-now for a worker's notice: task_status reads it"),
    n("watch_threads", Excluded, "the app's live transcript updates"),
    n("update_self", Confirm, "Updates pinned on the machine to the latest release and restarts it, which stops the sessions it runs.").wired(),
    n("sync_stopped", Read, "Why the machine stopped syncing projects and plugins, or null while it syncs.").wired(),
    n("reset_sync", Write, "Sets an unreadable sync state aside, so the machine's next sync starts over as a new machine's."),
    n("delegate", Excluded, "the hub's way to start a worker here for another machine: call delegate_task with target.machine").named("node_delegate"),
    n("remote_task", Excluded, "the hub's way to read a worker running here for another machine: call task_status").named("node_remote_task"),
    n("remote_ended", Excluded, "the hub's way to tell a machine its remote worker ended").named("node_remote_ended"),
    n("remote_asked", Excluded, "the hub's way to tell a machine its remote worker waits on the user").named("node_remote_asked"),
    n("state", Excluded, "the hub's way to read a machine's load and accounts before it refuses a delegation: call orchestrator_capabilities"),
    n("act_for", Excluded, "the hub's way to run an agent's act on its machine"),
    n("notice", Excluded, "the hub's way to tell a session what the user decided"),
    // The account, on the hub.
    h("get_settings", Read, "The account's integrations (who is connected to Linear, Slack with its bots by workspace, GitHub, Vercel, Upstash and Convex, never a key), its workflows, their last check, and box access.").wired(),
    h("connect_linear", Confirm, "Connects Linear. The user enters the key in the app; an agent never sends one.").wired().asks(&["key"]),
    h("disconnect_linear", Confirm, "Disconnects Linear.").wired(),
    h("linear_suggestions", Read, "Linear's labels, columns and teams, for a workflow's form.").wired(),
    h("connect_slack", Confirm, "Connects a Slack bot, the bot of one Slack app in one workspace, or updates the token of a bot already connected. The user enters the token in the app; an agent never sends one.").wired().asks(&["token"]),
    h("disconnect_slack", Confirm, "Disconnects one Slack bot: the optional `bot` argument is its id, and empty means the default bot, the first one connected. Refused while a workflow uses the bot.").wired(),
    h("slack_suggestions", Read, "The Slack channel names a workflow may watch through one bot: the optional `bot` argument is its id, and empty means the default bot.").wired(),
    h("connect_github", Confirm, "Connects GitHub. The user enters the token in the app; an agent never sends one.").wired().asks(&["token"]),
    h("disconnect_github", Confirm, "Disconnects GitHub.").wired(),
    h("github_repos", Read, "The GitHub repositories the account's token reaches, most recently pushed first.").wired(),
    h("connect_vercel", Confirm, "Connects Vercel's AI gateway. The user enters the key in the app; an agent never sends one.").wired().asks(&["key"]),
    h("disconnect_vercel", Confirm, "Disconnects Vercel.").wired(),
    h("vercel_models", Read, "The chat and decision models a workflow may use.").wired(),
    h("connect_upstash", Confirm, "Connects Upstash for boxes. The user enters the keys in the app; an agent never sends one.").wired().asks(&["key", "claude_token"]),
    h("disconnect_upstash", Confirm, "Deletes the account's boxes and disconnects Upstash.").wired(),
    h("list_schedules", Read, "Every scheduled task of the account as JSON, soonest first, with its next run (next_at) and last run.").wired(),
    h("save_schedule", Write, "Saves a scheduled task: leave id empty to create one, or send an existing task whole to edit it. schedule is {\"kind\":\"once\",\"at\":\"2026-11-13T09:00\"} or {\"kind\":\"cron\",\"expr\":\"0 9 * * 1-5\"} in tz, an IANA zone such as America/Sao_Paulo. target is {\"kind\":\"me\"}, orchestrator, captain {project}, or session {session}; action is {\"kind\":\"prompt\",\"text\":...} or workflow {id}. dry_run: true answers the next three run times in tz and stores nothing: read them back to the user. A task you create is active at once and the user is told; it may not repeat more often than every 15 minutes, and you may hold 20. A task that carries approvals (steps it may take without asking) waits for the user's approval.").wired().checks(crate::schedule::agent_check),
    h("pause_schedule", Write, "Pauses a scheduled task (paused: true) or starts it again, and answers it.").wired(),
    h("delete_schedule", Write, "Deletes a scheduled task and its run history. Deleting one the user created waits for the user's approval.").wired(),
    h("schedule_runs", Read, "The runs of scheduled task id, newest first, up to limit (20 when left out).").wired(),
    h("set_release_protection", Excluded, "whether tag and main protection exist, which only the user says: until then a task may not be pre-approved for a release"),
    h("set_box_access", Confirm, "Sets what the account's boxes may reach.").wired(),
    h("set_repo_insight", Confirm, "Turns on or off whether the account's global orchestrator may read its repositories with repo_overview, repo_read and repo_search.").wired(),
    h("answer_box", Excluded, "the user's yes or no to a box's question, which only they give, on the Upstash card in the app"),
    h("clone", Write, "Clones repo on machine with the account's GitHub token, or finds the clone it has, and answers the folder.").wired(),
    h("shared_projects", Read, "The account's synced projects, most recently shared first.").wired(),
    h("share_project", Write, "Offers a project, {key, name, repo}, to the account's other machines to clone.").wired().sends(&["project.key"]),
    h("sync_list", Excluded, SYNC),
    h("sync_push", Excluded, SYNC),
    h("sync_pull", Excluded, SYNC),
    h("sync_mark_executable", Excluded, SYNC),
    h("save_connector", Write, "Saves a workflow (a new one with no id, else the one with that id) and answers it as stored. A workflow with a session that skips permission prompts needs a session that skips them too.").wired(),
    h("delete_connector", Confirm, "Deletes a workflow.").wired(),
    h("generate_workflow", Write, "Drafts a workflow map from a plain description with a chat model.").wired(),
    h("run_workflow", Write, "Runs the saved workflow id now, with no Linear issue or Slack thread behind it, and answers the session it started: its machine and tab. The session has the setup the workflow gives its sessions and opens on your own machine; a workflow set to another machine or an Upstash Box is refused. It is your task: it counts against its project's workers-at-once limit, is held to the project's tier, effort, time limit and budget, and you get its notice when its first turn ends. A workflow that only sorts Slack messages or only answers threads another workflow hands it has nothing to run.").wired().starts_work(),
    h("open_box", Write, "Starts an interactive session on a new Upstash Box cloning repo, within the account's box limit and its Box budget: budget_usd may not pass it, and is it when left out. Without size it opens at the default box size that get_box_size answers. Once the box ends it counts as your delegated task in delegation_stats.").wired().starts_work(),
    h("box_status", Read, "The phase, age and budget of the box behind machine.").wired(),
    h("extend_box", Write, "Adds extra_usd to the budget of the box behind machine, which may not pass the account's Box budget for its project.").wired(),
    h("machines", Read, "Every machine of the account: id, name, OS, whether it is online, when it was last seen, its pinned version and update, and its box.").wired(),
    h("rename_machine", Write, "Renames the account's machine id to name.").wired(),
    h("remove_machine", Confirm, "Removes a machine from the account: its sessions stop showing, and a box's machine is deleted with its box.").wired(),
    h("pair_code", Confirm, "Makes a one-time code, and the command that uses it, to pair a new machine with the account. You get its expiry; the user gets the code.").wired().hides(&["code", "command"]),
    h("create_invite", Confirm, "Makes a single-use invite code that lets one person create an account on this hub. You get its id and expiry; the user gets the code.").wired().hides(&["code"]),
    h("invites", Read, "The hub's signup mode and the account's unused invite codes, named by their first characters.").wired(),
    h("revoke_invite", Confirm, "Stops an unused invite code from working.").wired(),
    h("import", Excluded, "a signed-out app's one-time move of its connectors"),
    h("stage", Write, "Moves an issue of one of the account's workflows to the stage's Linear column, with the account's Linear connection.").wired(),
    h("slack_reply", Write, "Replies in a Slack thread of one of the account's workflows, through the Slack bot that workflow uses.").wired(),
    h("linear_create", Write, "Files a Linear issue for a Slack thread of one of the account's workflows.").wired(),
    h("linear_take", Write, "Ties an existing Linear issue to a Slack thread of one of the account's workflows.").wired(),
    h("run_set", Excluded, "a value a workflow session sets for its own graph run, which only that session's machine sends"),
    h("external_agents", Read, "The hub's MCP endpoint and the external agents the account let in, with their scopes and projects.").wired(),
    h("agent_request", Excluded, "the consent page's own read: external_agents lists access"),
    h("approve_agent", Confirm, "Lets a pending external agent in with scopes on projects. The user's app gets the address that hands the agent its code.").wired().hides(&["redirect"]),
    h("deny_agent", Confirm, "Refuses a pending external agent.").wired().hides(&["redirect"]),
    h("revoke_agent", Confirm, "Ends an external agent's access at once.").wired(),
    h("relay", Excluded, "the global orchestrator's bridge between machines: it calls send_to_captain"),
    h("overview", Read, "Every project of the account on every machine with its Captain's state, what waits on the user, every machine, and every session.").wired(),
    h("read_captain", Read, "The latest replies of the Captain of the project at path on machine.").wired(),
    h("get_tastes", Read, "The account's way of working: the orchestrator's, the Captains' and each project's, with the delegation limits.").wired(),
    h("set_taste", Confirm, "Changes fields of one scope of the account's way of working.").wired(),
    h("reset_taste_field", Confirm, "Makes a field of one scope of the way of working inherit again.").wired(),
    h("import_tastes", Excluded, "a signed-out app's one-time move of its way of working"),
    h("get_box_size", Read, "The size a box cloning repo opens at when its call names none, and which setting chose it: call, project, account or built_in. Without repo, the account's. With size, what a call naming that size gets.").wired(),
    h("set_box_size", Confirm, "Sets the default box size, small, medium or large, of repo's project, or the account's without repo. Leaving size out clears it: the project follows the account's again, and the account's goes back to small.").wired(),
    h("read_session", Read, "Any session's state and last items, found on the account's online machines by its tab id, on machine when given.").wired(),
    h("repo_look", Excluded, "the global orchestrator's own repo tools: it calls repo_overview, repo_read and repo_search"),
    h("record_tasks", Excluded, "machines send their delegated tasks' records by themselves"),
    h("delegation_stats", Read, "How the account's delegated tasks went over the last days (1, 7 or 30), per model.").wired(),
    h("box_usage", Read, "The account's Upstash Box usage: every box the key lists with its CPU hours and compute cost, the totals of this UTC month, and the box hours of pinned's own delegated tasks.").wired(),
    h("delegate", Excluded, "a machine's way to run delegate_task on another machine or a box: call delegate_task"),
    h("remote_task", Excluded, "a machine's way to read its remote worker: call task_status"),
    h("remote_ended", Excluded, "a machine's way to report a remote worker's end"),
    h("place", Excluded, "a machine's way to have the hub pick the machine for a heavy worker: call delegate_task, which places it by the account's policy"),
    h("remote_asked", Excluded, "a machine's way to report that a remote worker waits on the user"),
    h("machine_states", Excluded, "a machine's way to read every machine's load for orchestrator_capabilities: call orchestrator_capabilities"),
    h("set_box_claude_token", Confirm, "Keeps a Claude token for boxes that run as a pinned account. The user enters the token in the app; an agent never sends one.").wired().asks(&["token"]),
    h("remove_box_claude_token", Confirm, "Forgets the Claude token kept for boxes that run as a pinned account.").wired(),
    h("mint_box_claude_token", Confirm, "Has machine run claude setup-token for its Claude account account, and keeps the long-lived token it prints for the boxes that run as that account. The user approves it twice: here, then in the browser that opens on that machine, signed in as that account. The token reaches no agent and no app.").wired(),
    h("cancel_box_claude_token", Write, "Stops a mint_box_claude_token that still waits on machine for account's approval in the browser.").wired(),
    h("session_marks", Read, "How the user filed each conversation of the account, by Claude session id: title (the name shown in place of its own), pin and order (places, lowest first), settled, unsettled and seen (unix seconds; no seen shows a finished turn as unread), snoozed (until and at, unix seconds), no_auto_settle, and archived.").wired(),
    h("mark_sessions", Write, "Changes marks of conversations, by Claude session id, in order, and answers the rows it touched. Each change is {session, mark}, mark one of {\"title\": \"Fix login\"} or {\"title\": null} to rename, {\"pin\": 0} or {\"pin\": null}, {\"order\": n}, {\"settled\": <unix s>} or null, {\"snoozed\": {\"until\": <unix s>, \"at\": <unix s>}} or null, {\"seen\": null} to mark unread, {\"no_auto_settle\": true}. Every device of the account sees it at once.").wired().checks(crate::marks::agent_check),
    h("archive_session", Write, "Archives the conversation in tab on machine: its tab closes, and Settings › Archive keeps it with its name and pin to bring back. Refused while its turn runs: stop_turn first.").wired(),
    h("auto_archive", Excluded, "a machine's own check archives its finished threads; an agent archives with archive_session"),
    h("unarchive_session", Write, "Brings an archived conversation back, by Claude session id: a new tab on its machine with its name and pin back, not started, and answers {machine, tab}. Resume it with start_claude, which holds it to your limits.").wired(),
    h("delete_session", Confirm, "Closes the conversation in tab on machine and drops its marks, so pinned forgets it for good. Claude Code keeps the transcript.").wired(),
    h("act", Excluded, "the surface itself: call act"),
    h("confirmations", Read, "The actions waiting on the user's approval, then the last 20 decided, with what each came to.").wired(),
    h("decide_confirmation", Excluded, "only the user decides, in the app"),
    h("unlock", Excluded, "the user's own password, which only the app sends"),
    h("agent_audit", Read, "The audit trail of agents' acts on this account, newest first: limit rows (50, at most 200) from unix second before and earlier.").wired(),
    h("list_mcp_servers", Read, "The account's own MCP servers, which every session pinned starts loads while enabled (Claude Code by --mcp-config, Codex by -c), alongside the servers its agent account configures itself, which keep a name they share: id, name, project (a repository, or every project), enabled, and transport (stdio command, args and env names, or http or sse url and header names). Never a secret's value.").wired(),
    h("save_mcp_server", Confirm, "Adds an MCP server to the account's list, enabled, or with id changes that one. transport is {type: stdio, command, args, env: [names]} or {type: http or sse, url, headers: [names]}; project is a repository like github.com/owner/repo, or left out for every project. The user types each env and header value in the app; an agent never sends one, and keys never go in the command, arguments or URL.").wired().asks(&["secrets"]),
    h("enable_mcp_server", Write, "Turns a server of the account's MCP list on (enabled: true) or off for the sessions started from now on.").wired(),
    h("remove_mcp_server", Confirm, "Removes a server and its secrets from the account's MCP list.").wired(),
    h("import_mcp_server", Confirm, "Adds the MCP server name that Claude Code account account on machine configures (as mcp_servers lists it, path for its local and project scope) to the account's list, for every project or for project. The user types each of its env and header values in the app.").wired().asks(&["secrets"]),
    h("load_mcp_servers", Excluded, "a machine's way to load the account's MCP servers, secrets and all, into the sessions it starts"),
    h("connect_convex", Confirm, "Connects a Convex deployment to repos, the account's projects as repo keys like github.com/owner/repo, whose sessions then read its data. The user pastes the deploy key in the app; an agent never sends one.").wired().asks(&["key"]),
    h("set_convex_repos", Confirm, "Sets the projects, as repo keys, whose sessions read the connected Convex deployment id. A project moves off any other deployment.").wired(),
    h("disconnect_convex", Confirm, "Disconnects the Convex deployment id and forgets its key.").wired(),
    h("convex_tables", Excluded, "the Read Convex data step's table list in the app: a session reads Convex through its convex_ tools"),
    h("convex", Excluded, "a session's Convex read, which its machine sends: call the convex_ tools"),
];

/// A signed-in client's answer to a call only the account's owner makes, until it unlocks.
pub const LOCKED: &str = "unlock with your password in the app to do this";

/// Whether a signed-in client must have unlocked with the password to run `cmd` on `side`: a
/// decision always, and a `confirm` action on a `locked` login. The app keeps its login on disk,
/// where any process of the same user reads it; the unlock lives in the app's memory only.
pub fn owner_only(side: Side, cmd: &str, locked: bool) -> bool {
    if side == Side::Hub && cmd == "decide_confirmation" {
        return true;
    }
    locked && of_cmd(side, cmd).is_some_and(|action| action.class == Confirm)
}

/// Hub actions that start work and that the hub holds to the limits itself: the server's `act`
/// runs each through its own limits path, and refuses one it has no path for.
pub const HUB_LIMITS: &[&str] = &["open_box", "run_workflow"];

pub fn find(name: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|action| action.name == name)
}

pub fn of_cmd(side: Side, cmd: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|action| action.side == side && action.cmd == cmd)
}

/// The `cmd` a call goes by on the wire.
pub fn cmd_of<T: Serialize>(call: &T) -> String {
    serde_json::to_value(call).ok().and_then(|v| v["cmd"].as_str().map(str::to_owned)).unwrap_or_default()
}

pub enum Parsed {
    Node(Call),
    Hub(HubCall),
}

/// The call `action` names with `args`, its fields. `args` null reads as no fields.
pub fn parse(action: &Action, args: &Value) -> Result<Parsed, String> {
    let mut fields = match args {
        Value::Null => Map::new(),
        Value::Object(fields) => fields.clone(),
        _ => return Err(format!("{}: args must be an object of the action's fields", action.name)),
    };
    fields.insert("cmd".into(), action.cmd.into());
    let call = Value::Object(fields);
    let parsed = match action.side {
        Side::Node => serde_json::from_value(call).map(Parsed::Node),
        Side::Hub => serde_json::from_value(call).map(Parsed::Hub),
    };
    let parsed = parsed.map_err(|e| format!("{}: {e}", action.name))?;
    if let Some(check) = action.check {
        check(&parsed)?;
    }
    Ok(parsed)
}

/// `args` without the action's asked fields, as a confirmation keeps them.
pub fn without_asked(action: &Action, args: &Value) -> Value {
    match args {
        Value::Object(fields) => Value::Object(fields.iter().filter(|(name, _)| !action.asks.contains(&name.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()),
        other => other.clone(),
    }
}

/// `args` with each asked text field the agent left out blank, so `parse` checks the rest of
/// them. An asked object, like `save_mcp_server`'s `secrets`, defaults to empty by itself.
pub fn with_blank_asks(action: &Action, args: &Value) -> Value {
    match args {
        Value::Null | Value::Object(_) if !action.asks.is_empty() => {
            let mut fields = args.as_object().cloned().unwrap_or_default();
            let schema = args_schema(action);
            for field in action.asks.iter().filter(|field| schema["properties"][**field]["type"] != "object") {
                fields.entry(*field).or_insert_with(|| Value::String(String::new()));
            }
            Value::Object(fields)
        }
        other => other.clone(),
    }
}

/// The stored arguments of a confirmation with what the user typed in the app for its asked
/// fields: only those, and every one the call needs, not blank. A field of an asked object is
/// named by its path, as `secrets.env.API_KEY`.
pub fn with_entered(action: &Action, args: &Value, entered: &BTreeMap<String, String>) -> Result<Value, String> {
    let name = action.name;
    if action.asks.is_empty() && !entered.is_empty() {
        return Err(format!("{name} takes nothing typed in the app"));
    }
    if let Some(field) = entered.keys().find(|field| !action.asks.contains(&field.split('.').next().unwrap_or_default())) {
        return Err(format!("{name} does not take {field}"));
    }
    let mut fields = args.as_object().cloned().unwrap_or_default();
    for (path, value) in entered {
        let (last, parents) = path.rsplit_once('.').map_or((path.as_str(), None), |(parents, last)| (last, Some(parents)));
        let mut at = &mut fields;
        for part in parents.into_iter().flat_map(|p| p.split('.')) {
            let slot = at.entry(part).or_insert_with(|| Value::Object(Map::new()));
            if !slot.is_object() {
                *slot = Value::Object(Map::new());
            }
            at = slot.as_object_mut().expect("made an object");
        }
        at.insert(last.to_owned(), Value::String(value.clone()));
    }
    let schema = args_schema(action);
    let required = schema["required"].as_array().map(Vec::as_slice).unwrap_or_default();
    let blank = |field: &str| fields.get(field).and_then(Value::as_str).is_none_or(|value| value.trim().is_empty());
    if let Some(field) = action.asks.iter().find(|field| required.iter().any(|r| r == **field) && blank(field)) {
        return Err(format!("{name} needs {field}, which the user types in the app"));
    }
    Ok(Value::Object(fields))
}

/// `text` with secrets masked and each value the user typed in the app as `[hidden]`.
pub fn hide_entered(text: &str, entered: &BTreeMap<String, String>) -> String {
    let mut text = mask_secrets(text);
    for typed in entered.values().map(|value| value.trim()).filter(|typed| !typed.is_empty()) {
        text = text.replace(typed, HIDDEN);
    }
    text
}

/// `value` with each value the user typed in the app as `[hidden]` in every string.
pub fn hide_entered_in(value: Value, entered: &BTreeMap<String, String>) -> Value {
    match value {
        Value::String(text) => Value::String(hide_entered(&text, entered)),
        Value::Array(items) => Value::Array(items.into_iter().map(|item| hide_entered_in(item, entered)).collect()),
        Value::Object(fields) => Value::Object(fields.into_iter().map(|(name, field)| (name, hide_entered_in(field, entered))).collect()),
        other => other,
    }
}

/// One action as `actions` lists it.
#[derive(Serialize, Debug)]
pub struct Listed {
    pub name: &'static str,
    pub class: Class,
    pub on: &'static str,
    pub description: &'static str,
    pub args: Value,
}

/// Every wired action that isn't excluded, with the JSON Schema of its arguments. `search` keeps
/// those whose name or description holds each of its words.
pub fn listing(search: Option<&str>) -> Vec<Listed> {
    let words: Vec<String> = search.unwrap_or("").split_whitespace().map(str::to_lowercase).collect();
    ACTIONS
        .iter()
        .filter(|action| action.wired && action.class != Excluded)
        .filter(|action| {
            let text = format!("{} {}", action.name.replace('_', " "), action.about).to_lowercase();
            words.iter().all(|word| text.contains(word.as_str()))
        })
        .map(|action| Listed { name: action.name, class: action.class, on: action.on(), description: action.about, args: args_schema(action) })
        .collect()
}

fn args_schema(action: &Action) -> Value {
    let schemas = match action.side {
        Side::Node => node_schemas(),
        Side::Hub => hub_schemas(),
    };
    schemas.get(action.cmd).cloned().unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} }))
}

fn node_schemas() -> &'static BTreeMap<String, Value> {
    static SCHEMAS: OnceLock<BTreeMap<String, Value>> = OnceLock::new();
    SCHEMAS.get_or_init(variant_schemas::<Call>)
}

fn hub_schemas() -> &'static BTreeMap<String, Value> {
    static SCHEMAS: OnceLock<BTreeMap<String, Value>> = OnceLock::new();
    SCHEMAS.get_or_init(variant_schemas::<HubCall>)
}

/// Each variant of a `cmd`-tagged enum by its `cmd`: the schema of its fields, without `cmd`, with
/// every `$ref` written out.
fn variant_schemas<T: JsonSchema>() -> BTreeMap<String, Value> {
    let root = serde_json::to_value(SchemaSettings::draft2020_12().for_deserialize().into_generator().into_root_schema_for::<T>()).expect("schema");
    let defs = root.get("$defs").and_then(Value::as_object).cloned().unwrap_or_default();
    let variants = root.get("oneOf").or_else(|| root.get("anyOf")).and_then(Value::as_array).cloned().unwrap_or_default();
    variants
        .into_iter()
        .filter_map(|variant| {
            let cmd = tag_of(&variant)?;
            let mut fields = inline(variant, &defs, 0);
            if let Some(object) = fields.as_object_mut() {
                object.shift_remove("description");
                if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
                    properties.shift_remove("cmd");
                }
                if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
                    required.retain(|name| name != "cmd");
                }
            }
            Some((cmd, fields))
        })
        .collect()
}

fn tag_of(variant: &Value) -> Option<String> {
    let cmd = &variant["properties"]["cmd"];
    if let Some(tag) = cmd["const"].as_str().or_else(|| cmd["enum"][0].as_str()) {
        return Some(tag.to_owned());
    }
    ["allOf", "anyOf", "oneOf"].iter().filter_map(|key| variant[key].as_array()).flatten().find_map(tag_of)
}

fn inline(value: Value, defs: &Map<String, Value>, depth: usize) -> Value {
    match value {
        Value::Object(mut object) => {
            if let Some(name) = object.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix("#/$defs/")).map(str::to_owned) {
                object.shift_remove("$ref");
                let target = match defs.get(&name) {
                    Some(def) if depth < 8 => inline(def.clone(), defs, depth + 1),
                    _ => serde_json::json!({ "type": "object" }),
                };
                if let Value::Object(target) = target {
                    for (key, field) in target {
                        object.entry(key).or_insert(field);
                    }
                }
            }
            Value::Object(object.into_iter().map(|(key, field)| (key, inline(field, defs, depth))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(|item| inline(item, defs, depth)).collect()),
        other => other,
    }
}

/// An agent session's `act`, as its machine passes it to the hub. The hub reads the session's role
/// and Claude session from the tabs that machine reported, never from here.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct AgentAct {
    pub tab: TabId,
    pub action: String,
    /// The action's fields, as `actions` describes them.
    #[serde(default)]
    pub args: Value,
    /// Makes a repeat answer the first outcome instead of running again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// The machine a `Call` runs on, an id or a name; the session's own when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// What `act` came to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ActAnswer {
    /// It ran. `result` is what the call answered, with secrets and the action's hidden fields out.
    Done { result: Value },
    /// It waits on the user's approval in the app as confirmation `id`.
    Pending { id: String, next: String },
    /// The account already ran an act with this `request_id`, which came to `earlier` at unix time `at`.
    Repeat { earlier: AuditOutcome, at: i64 },
}

pub const PENDING_NEXT: &str = "The user approves or denies this in the app. A notice reaches this session when they decide.";

/// The part the session that acted plays, as its tab shows it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActorRole {
    GlobalOrchestrator,
    Captain,
    Worker,
    TasteChat,
    Workflow,
    Session,
}

impl ActorRole {
    pub fn of(tab: &TabView) -> ActorRole {
        use crate::agent::Role;
        match tab {
            t if t.orchestrator_global => ActorRole::GlobalOrchestrator,
            TabView { role: Some(Role::Orchestrator), .. } => ActorRole::Captain,
            TabView { role: Some(Role::Worker), .. } => ActorRole::Worker,
            t if t.taste_chat => ActorRole::TasteChat,
            t if t.connector.is_some() => ActorRole::Workflow,
            _ => ActorRole::Session,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ActorRole::GlobalOrchestrator => "global_orchestrator",
            ActorRole::Captain => "captain",
            ActorRole::Worker => "worker",
            ActorRole::TasteChat => "taste_chat",
            ActorRole::Workflow => "workflow",
            ActorRole::Session => "session",
        }
    }

    /// How a person reads the role, as "Run now by the Captain …".
    pub fn label(self) -> &'static str {
        match self {
            ActorRole::GlobalOrchestrator => "the global orchestrator",
            ActorRole::Captain => "the Captain",
            ActorRole::Worker => "the worker",
            ActorRole::TasteChat => "the taste chat",
            ActorRole::Workflow => "the workflow session",
            ActorRole::Session => "the session",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Done,
    Failed,
    Refused,
    Pending,
    Approved,
    Denied,
}

impl AuditOutcome {
    pub fn name(self) -> &'static str {
        match self {
            AuditOutcome::Done => "done",
            AuditOutcome::Failed => "failed",
            AuditOutcome::Refused => "refused",
            AuditOutcome::Pending => "pending",
            AuditOutcome::Approved => "approved",
            AuditOutcome::Denied => "denied",
        }
    }

    pub fn parse(name: &str) -> Option<AuditOutcome> {
        [AuditOutcome::Done, AuditOutcome::Failed, AuditOutcome::Refused, AuditOutcome::Pending, AuditOutcome::Approved, AuditOutcome::Denied].into_iter().find(|o| o.name() == name)
    }
}

/// One row of the audit trail: an agent's act, a refusal, or the user's decision on a confirmation.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct AuditEntry {
    pub id: String,
    /// Unix seconds.
    pub at: i64,
    pub machine: String,
    /// Empty for a machine since removed.
    #[serde(default)]
    pub machine_name: String,
    pub tab: TabId,
    pub role: ActorRole,
    pub action: String,
    /// The arguments as compact JSON, secrets hidden and long strings cut (`summary`).
    pub args: String,
    pub outcome: AuditOutcome,
    /// The error or refusal, or what a decided action came to.
    #[serde(default)]
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// The scheduled task the act is about: one it saved, paused or deleted, or later one a run did it for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approved,
    Denied,
}

/// A `confirm` action an agent asked for, waiting on the user or decided.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Confirmation {
    pub id: String,
    /// Unix seconds it was asked.
    pub at: i64,
    pub machine: String,
    #[serde(default)]
    pub machine_name: String,
    pub tab: TabId,
    /// The asking session's title when it asked.
    #[serde(default)]
    pub title: String,
    pub role: ActorRole,
    pub action: String,
    /// What the action does, from the manifest.
    #[serde(default)]
    pub about: String,
    /// The exact arguments it runs with.
    pub args: Value,
    /// The machine `act` named for an action on a machine, an id or a name; the asking session's own
    /// machine when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Decision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<i64>,
    /// What running it came to: `done`, or `failed: <error>`. Absent until decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// The answer to the user's decision: the action's own answer, unredacted, for an approved one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Decided {
    pub decision: Decision,
    pub outcome: String,
    #[serde(default)]
    pub result: Value,
}

/// `HubCall::Unlock`'s answer. The app keeps `token` in memory and sends it again in
/// `ClientFrame::Auth` after a reconnect; it works for that login until `until`, unix seconds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Unlocked {
    pub token: String,
    pub until: i64,
}

/// What the audit row of a call that ran says beyond its arguments: that it loosened a session's
/// permissions.
pub fn widening(call: &Parsed) -> Option<String> {
    if skips_prompts(call) {
        return Some("loosened permissions: the saved workflow runs a session that skips its permission prompts".into());
    }
    match call {
        Parsed::Node(Call::SetBypass { tab, on: true, .. }) => Some(format!("loosened permissions: {tab} now skips its permission prompts")),
        Parsed::Node(Call::OpenTab { path, bypass: true }) => Some(format!("loosened permissions: the new tab in {} skips its permission prompts", path.as_deref().unwrap_or("the default folder"))),
        Parsed::Node(Call::DeviceAgents { allow: Some(true) }) => Some("loosened permissions: new sessions on this machine may drive its simulators and emulators".into()),
        _ => None,
    }
}

/// Whether `call` saves a workflow with a session that skips its permission prompts. It runs later,
/// outside the saving session, so a session that asks for its own prompts may not save one.
pub fn skips_prompts(call: &Parsed) -> bool {
    fn walk(value: &Value) -> bool {
        match value {
            Value::Object(fields) => fields.iter().any(|(name, field)| (matches!(name.as_str(), "permissionMode" | "permission") && field == "bypassPermissions") || walk(field)),
            Value::Array(items) => items.iter().any(walk),
            _ => false,
        }
    }
    match call {
        Parsed::Hub(HubCall::SaveConnector { connector }) => serde_json::to_value(connector).is_ok_and(|connector| walk(&connector)),
        _ => false,
    }
}

/// The text a session gets when the user decides its confirmation.
pub fn notice_text(confirmation: &Confirmation, result: &Value) -> String {
    let action = &confirmation.action;
    let id = &confirmation.id;
    match confirmation.decision {
        Some(Decision::Denied) => format!("[pinned] The user denied your {action} request ({id}). It did not run."),
        _ => {
            let outcome = confirmation.outcome.as_deref().unwrap_or("done");
            let shown = match result {
                Value::Null => String::new(),
                result => format!(" Result: {}", cut(&result.to_string(), 1000)),
            };
            format!("[pinned] The user approved your {action} request ({id}): {outcome}.{shown}")
        }
    }
}

pub(crate) fn secret_name(name: &str, sending: bool) -> bool {
    let name = name.to_ascii_lowercase();
    ["token", "password", "secret", "secrets", "api_key", "claude_token"].contains(&name.as_str())
        || ["_token", "_secret", "_password"].iter().any(|end| name.ends_with(end))
        || (sending && (name == "key" || name.ends_with("_key")))
}

/// The first field of `args`, at any depth, that holds a password, key or token, which an agent
/// never sends: text in a field with a secret's name, or anywhere inside one, as `secrets`. `plain`
/// lists the dotted paths an action's arguments may carry under such a name.
pub fn secret_arg(args: &Value, plain: &[&str]) -> Option<String> {
    find_secret(args, plain, "")
}

fn find_secret(args: &Value, plain: &[&str], at: &str) -> Option<String> {
    match args {
        Value::Object(fields) => fields.iter().find_map(|(name, value)| {
            let path = if at.is_empty() { name.clone() } else { format!("{at}.{name}") };
            match value {
                Value::String(text) if secret_name(name, true) && !text.trim().is_empty() && !plain.contains(&path.as_str()) => Some(path),
                Value::Object(_) | Value::Array(_) if secret_name(name, true) && holds_text(value) && !plain.contains(&path.as_str()) => Some(path),
                value => find_secret(value, plain, &path),
            }
        }),
        Value::Array(items) => items.iter().find_map(|item| find_secret(item, plain, at)),
        _ => None,
    }
}

fn holds_text(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Object(fields) => fields.values().any(holds_text),
        Value::Array(items) => items.iter().any(holds_text),
        _ => false,
    }
}

pub const SECRET_REFUSAL: &str = "an agent never sends a password, key or token";

/// `value` with secrets out: the values of secret fields and of `hides`, and anything in a string
/// shaped like a key or token, become `[hidden]`.
pub fn redact(value: Value, hides: &[&str]) -> Value {
    scrub(value, hides, false)
}

fn scrub(value: Value, hides: &[&str], sending: bool) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(name, field)| {
                    let hidden = hides.contains(&name.as_str()) || secret_name(&name, sending);
                    let field = match field {
                        Value::String(text) if hidden => Value::String(if text.is_empty() { text } else { HIDDEN.into() }),
                        kept @ (Value::Null | Value::Bool(_) | Value::Number(_)) if hidden => kept,
                        _ if hidden => Value::String(HIDDEN.into()),
                        field => scrub(field, hides, sending),
                    };
                    (name, field)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(|item| scrub(item, hides, sending)).collect()),
        Value::String(text) => Value::String(mask_secrets(&text)),
        other => other,
    }
}

const HIDDEN: &str = "[hidden]";

/// The arguments as the audit trail keeps them: compact JSON, secrets hidden, each string cut at
/// 200 characters and the whole at 1000.
pub fn summary(args: &Value) -> String {
    fn shorten(value: Value) -> Value {
        match value {
            Value::String(text) => Value::String(cut(&text, 200)),
            Value::Array(items) => Value::Array(items.into_iter().map(shorten).collect()),
            Value::Object(fields) => Value::Object(fields.into_iter().map(|(k, v)| (k, shorten(v))).collect()),
            other => other,
        }
    }
    let args = if args.is_null() { Value::Object(Map::new()) } else { args.clone() };
    cut(&shorten(scrub(args, &[], true)).to_string(), 1000)
}

fn cut(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

/// Prefixes keys and tokens of the services pinned and its users reach start with.
const SECRET_PREFIXES: [&str; 13] = ["sk-", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "lin_api_", "xoxb-", "xoxp-", "xoxa-", "xoxr-", "AKIA"];

/// `text` with anything shaped like a key, a token or a private key replaced by `[hidden]`.
pub fn mask_secrets(text: &str) -> String {
    let token_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut boundary = true;
    while let Some(c) = rest.chars().next() {
        if boundary {
            if let Some(len) = pem_block(rest).or_else(|| jwt(rest)).or_else(|| convex_key(rest)).or_else(|| {
                SECRET_PREFIXES.iter().find_map(|prefix| {
                    let tail = rest.strip_prefix(prefix)?;
                    let run = tail.chars().take_while(|c| token_char(*c)).count();
                    let long_enough = if *prefix == "AKIA" { run >= 16 && tail.chars().take(16).all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) } else { run >= 16 };
                    long_enough.then(|| prefix.len() + tail.chars().take(run).map(char::len_utf8).sum::<usize>())
                })
            }) {
                out.push_str(HIDDEN);
                rest = &rest[len..];
                boundary = false;
                continue;
            }
        }
        out.push(c);
        boundary = !token_char(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// A Convex deploy key: `dev:`, `prod:` or `preview:`, a name, `|` and a token of 16 characters or
/// more. With no prefix the name must read as a deployment's and the token be 40 characters or
/// more, so a shell pipe, a regex alternation or a table rule is not taken for a key.
fn convex_key(text: &str) -> Option<usize> {
    let named = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-');
    let token = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=');
    let prefix = ["dev:", "prod:", "preview:"].into_iter().find(|p| text.starts_with(p)).unwrap_or("");
    let rest = &text[prefix.len()..];
    let name = rest.chars().take_while(|c| named(*c) || (*c == ':' && prefix == "preview:")).count();
    let tail = rest[name..].strip_prefix('|')?;
    let run = tail.chars().take_while(|c| token(*c)).count();
    if prefix.is_empty() && !deployment_name(&rest[..name]) {
        return None;
    }
    let enough = if prefix.is_empty() { 40 } else { 16 };
    (name > 0 && run >= enough).then(|| prefix.len() + name + 1 + run)
}

/// `happy-otter-123` or `anonymous-agent`: words joined by hyphens, the last one maybe a number.
fn deployment_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('-').collect();
    let word = |part: &str| part.starts_with(|c: char| c.is_ascii_alphabetic());
    let number = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    parts.len() >= 2 && parts.iter().enumerate().all(|(at, part)| word(part) || (at + 1 == parts.len() && number(part)))
}

fn jwt(text: &str) -> Option<usize> {
    if !text.starts_with("eyJ") {
        return None;
    }
    let run: usize = text.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')).map(char::len_utf8).sum();
    (run >= 30 && text[..run].matches('.').count() >= 2).then_some(run)
}

fn pem_block(text: &str) -> Option<usize> {
    let header = text.strip_prefix("-----BEGIN ")?;
    let line_end = header.find("-----")?;
    if !header[..line_end].ends_with("PRIVATE KEY") {
        return None;
    }
    let end = text.find("-----END ")?;
    let close = text[end + 9..].find("-----")? + end + 9 + 5;
    Some(close)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    #[test]
    fn every_wire_call_has_one_action() {
        let wire: BTreeSet<(bool, String)> = variant_schemas::<Call>().into_keys().map(|cmd| (false, cmd)).chain(variant_schemas::<HubCall>().into_keys().map(|cmd| (true, cmd))).collect();
        let listed: BTreeSet<(bool, String)> = ACTIONS.iter().map(|a| (a.side == Side::Hub, a.cmd.to_owned())).collect();
        let side = |hub: bool| if hub { "HubCall" } else { "Call" };
        let missing: Vec<String> = wire.difference(&listed).map(|(hub, cmd)| format!("{}::{cmd}", side(*hub))).collect();
        assert!(missing.is_empty(), "no entry in surface::ACTIONS for {missing:?}: decide whether agents may read, write, confirm or never run it");
        let stale: Vec<String> = listed.difference(&wire).map(|(hub, cmd)| format!("{}::{cmd}", side(*hub))).collect();
        assert!(stale.is_empty(), "surface::ACTIONS names calls the wire no longer has: {stale:?}");
        assert_eq!(ACTIONS.len(), wire.len(), "one entry per call");
        assert!(wire.len() > 160, "the schemas list every variant: {}", wire.len());
    }

    #[test]
    fn a_decision_always_needs_the_unlock_and_a_confirm_action_needs_it_on_a_locked_login() {
        let needs = |side, cmd, locked| owner_only(side, cmd, locked);
        assert!(needs(Side::Hub, "decide_confirmation", false) && needs(Side::Hub, "decide_confirmation", true));
        assert!(needs(Side::Hub, "remove_machine", true) && !needs(Side::Hub, "remove_machine", false), "an older app's login keeps its calls");
        assert!(needs(Side::Node, "update_self", true) && needs(Side::Hub, "approve_agent", true) && needs(Side::Hub, "set_taste", true));
        assert!(!needs(Side::Hub, "machines", true) && !needs(Side::Hub, "rename_machine", true) && !needs(Side::Node, "write_input", true));
        assert!(!needs(Side::Hub, "unlock", true), "unlocking needs no unlock");
        assert!(!needs(Side::Node, "decide_confirmation", true), "no machine call is a decision");
        assert!(!needs(Side::Hub, "answer_box", true) && !needs(Side::Hub, "extend_box", true), "a box follows its access setting and Box budget, never the password");
        for cmd in ["set_box_access", "connect_upstash", "disconnect_upstash", "set_box_claude_token", "remove_box_claude_token", "connect_convex", "set_convex_repos", "disconnect_convex"] {
            assert!(needs(Side::Hub, cmd, true), "changing who may use a box, or its credentials, still needs the password: {cmd}");
        }
        let confirm: Vec<&str> = ACTIONS.iter().filter(|a| needs(a.side, a.cmd, true)).map(|a| a.name).collect();
        assert_eq!(confirm.len(), ACTIONS.iter().filter(|a| a.class == Confirm).count() + 1, "every confirm action and the decision: {confirm:?}");
        assert!(include_str!("../../../ui/hub.js").contains(&format!("const LOCKED = \"{LOCKED}\";")), "the app asks for the password on this exact refusal");
    }

    #[test]
    fn names_are_unique_and_a_shared_cmd_names_the_node_call_node_cmd() {
        let names: BTreeSet<&str> = ACTIONS.iter().map(|a| a.name).collect();
        assert_eq!(names.len(), ACTIONS.len(), "two actions share a name");
        for action in ACTIONS {
            let shared = ACTIONS.iter().any(|other| other.cmd == action.cmd && other.side != action.side);
            let expected = if shared && action.side == Side::Node { format!("node_{}", action.cmd) } else { action.cmd.to_owned() };
            assert_eq!(action.name, expected);
        }
        assert_eq!(find("read_session").map(|a| a.side), Some(Side::Hub));
        assert_eq!(find("node_read_session").map(|a| (a.side, a.cmd)), Some((Side::Node, "read_session")));
        assert!(find("node_tabs").is_none());
    }

    #[test]
    fn excluded_never_runs_every_read_runs_and_hub_work_runs_only_through_a_limits_path() {
        for action in ACTIONS {
            assert!(!(action.class == Excluded && (action.wired || action.starts_work)), "{} is excluded", action.name);
            assert!(action.class != Read || action.wired, "{} is a read that is not wired", action.name);
            let hub_work = action.side == Side::Hub && action.starts_work && action.wired;
            assert!(!hub_work || HUB_LIMITS.contains(&action.name), "{} starts work on the hub: name it in HUB_LIMITS and give it a limits path in the server's act", action.name);
            assert!(!action.about.is_empty(), "{} says nothing", action.name);
        }
        for name in HUB_LIMITS {
            let action = find(name).unwrap_or_else(|| panic!("HUB_LIMITS names no action {name}"));
            assert!(action.side == Side::Hub && action.starts_work, "{name} is no hub action that starts work");
        }
        let wired: Vec<&str> = ACTIONS.iter().filter(|a| a.wired && a.class != Read).map(|a| a.name).collect();
        assert_eq!(
            wired,
            [
                "open_tab", "open_scratch", "start_claude", "set_bypass", "set_model", "restart_claude", "stop_turn", "set_folder", "create_project",
                "publish_project", "make_folder", "add_account", "set_account_key", "remove_account", "set_project_account", "open_orchestrator",
                "enqueue", "open_global_orchestrator", "set_global_orchestrator_settings", "memory_save", "memory_forget", "sign_in",
                "switch_account", "prompt", "regenerate_title", "save_skill", "set_usage_prices", "save_project_scripts", "set_workspace",
                "device_hub", "device_agents", "device", "rewind", "fork", "update_self", "connect_linear", "disconnect_linear", "connect_slack",
                "disconnect_slack", "connect_github", "disconnect_github", "connect_vercel", "disconnect_vercel", "connect_upstash",
                "disconnect_upstash", "set_box_access", "set_repo_insight", "clone", "share_project", "save_connector", "delete_connector",
                "generate_workflow", "run_workflow", "open_box", "extend_box", "rename_machine", "remove_machine", "pair_code", "create_invite", "revoke_invite",
                "stage", "slack_reply", "linear_create", "linear_take", "approve_agent", "deny_agent", "revoke_agent", "set_taste",
                "reset_taste_field", "set_box_size", "set_box_claude_token", "remove_box_claude_token", "mint_box_claude_token", "cancel_box_claude_token", "mark_sessions", "archive_session", "unarchive_session",
                "delete_session",
                "save_mcp_server", "enable_mcp_server", "remove_mcp_server", "import_mcp_server",
                "connect_convex", "set_convex_repos", "disconnect_convex",
            ]
        );
        for name in ["set_taste", "reset_taste_field", "set_box_size", "delete_connector"] {
            assert_eq!(find(name).map(|a| a.class), Some(Confirm), "{name} carries limits or deletes, so the user approves it");
        }
        assert!(find("enqueue").is_some_and(|a| a.starts_work), "a message that starts a Captain goes through the limits");
        assert!(find("run_workflow").is_some_and(|a| a.starts_work && a.class == Write), "a Run now goes through the limits, with no approval");
        assert_eq!(find("run_now").map(|a| a.class), Some(Excluded), "only the hub starts a Run now on a machine");
    }

    #[test]
    fn a_credential_action_is_a_confirm_that_asks_the_user_for_secret_fields_of_its_own_call() {
        let asking: Vec<String> = ACTIONS.iter().filter(|a| !a.asks.is_empty()).map(|a| format!("{}: {}", a.name, a.asks.join(" "))).collect();
        assert_eq!(
            asking,
            [
                "add_account: key",
                "set_account_key: key",
                "connect_linear: key",
                "connect_slack: token",
                "connect_github: token",
                "connect_vercel: key",
                "connect_upstash: key claude_token",
                "set_box_claude_token: token",
                "save_mcp_server: secrets",
                "import_mcp_server: secrets",
                "connect_convex: key",
            ]
        );
        for action in ACTIONS.iter().filter(|a| !a.asks.is_empty()) {
            assert_eq!(action.class, Confirm, "{} asks the user, so only a decision runs it", action.name);
            let fields = args_schema(action)["properties"].as_object().cloned().unwrap_or_default();
            for field in action.asks {
                assert!(fields.contains_key(*field), "{} has no field {field}", action.name);
                let sent = Value::Object([(field.to_string(), json!("typed"))].into_iter().collect());
                assert_eq!(secret_arg(&sent, &[]).as_deref(), Some(*field), "an agent could send {}'s {field}", action.name);
            }
        }
    }

    #[test]
    fn files_on_another_machine_are_wired_reads_that_run_only_elsewhere() {
        let elsewhere: Vec<&str> = ACTIONS.iter().filter(|a| a.elsewhere).map(|a| a.name).collect();
        assert_eq!(elsewhere, ["diff", "files", "read_file", "search"]);
        for name in elsewhere {
            let action = find(name).unwrap();
            assert_eq!((action.side, action.class, action.wired, action.on()), (Side::Node, Read, true, "machine"), "{name}");
            assert!(action.about.contains("another machine of the account"), "{name}: {}", action.about);
        }
        assert!(!find("write_file").unwrap().elsewhere && find("write_file").unwrap().class == Excluded);
    }

    /// One test per action P3b wires: its class and side, and that what an agent sends for it
    /// holds no secret and parses into its own call. A credential action refuses each field the
    /// user types in the app, and an action that hides fields of its answer shows them as hidden.
    mod p3b {
        use super::*;

        pub(super) fn wired(name: &str, class: Class, on: &str, starts_work: bool, args: Value) {
            let action = find(name).unwrap_or_else(|| panic!("no action {name}"));
            assert_eq!((action.class, action.wired, action.on(), action.starts_work), (class, true, on, starts_work), "{name}");
            assert_eq!(secret_arg(&args, action.sends), None, "{name}: an agent sends no secret");
            let cmd = match parse(action, &with_blank_asks(action, &args)) {
                Ok(Parsed::Node(call)) => cmd_of(&call),
                Ok(Parsed::Hub(call)) => cmd_of(&call),
                Err(e) => panic!("{e}"),
            };
            assert_eq!(cmd, action.cmd, "{name}");
            for field in action.asks {
                let mut sent = args.as_object().cloned().unwrap_or_default();
                sent.insert(field.to_string(), json!("typed-by-agent"));
                assert_eq!(secret_arg(&Value::Object(sent.clone()), &[]).as_deref(), Some(*field), "{name}: an agent sending {field} is refused");
                assert!(!summary(&Value::Object(sent)).contains("typed-by-agent"), "{name}: the audit row hides {field}");
                let blank = Value::Object([(field.to_string(), json!(""))].into_iter().collect());
                assert_eq!(without_asked(action, &blank), json!({}), "{name}: the confirmation keeps no {field}");
            }
            if !action.hides.is_empty() {
                let answer = Value::Object(action.hides.iter().map(|field| (field.to_string(), json!("only-the-user-sees-this"))).collect());
                assert!(!redact(answer, action.hides).to_string().contains("only-the-user-sees-this"), "{name}: the agent's answer hides {:?}", action.hides);
            }
        }

        macro_rules! wired {
            ($($name:ident: $class:ident on $on:literal => $args:tt;)*) => {
                $(#[test]
                fn $name() {
                    wired(stringify!($name), $class, $on, false, json!($args));
                })*
            };
        }

        wired! {
            add_account: Confirm on "machine" => { "provider": "claude", "name": "Side" };
            set_account_key: Confirm on "machine" => { "account": "go" };
            remove_account: Confirm on "machine" => { "account": "side" };
            sign_in: Confirm on "machine" => { "account": "work" };
            device_hub: Write on "machine" => {};
            device_agents: Write on "machine" => { "allow": true };
            device: Write on "machine" => { "op": "list" };
            update_self: Confirm on "machine" => {};
            read_file: Read on "machine" => { "tab": "tab-1", "path": "src/a.rs" };
            search: Read on "machine" => { "tab": "tab-1", "query": "fn main" };
            files: Read on "machine" => { "tab": "tab-1" };
            diff: Read on "machine" => { "tab": "tab-1", "scope": "uncommitted" };
            mcp_servers: Read on "machine" => { "path": "/w/site" };
            connect_linear: Confirm on "hub" => {};
            disconnect_linear: Confirm on "hub" => {};
            connect_slack: Confirm on "hub" => {};
            disconnect_slack: Confirm on "hub" => {};
            connect_github: Confirm on "hub" => {};
            disconnect_github: Confirm on "hub" => {};
            connect_vercel: Confirm on "hub" => {};
            disconnect_vercel: Confirm on "hub" => {};
            connect_upstash: Confirm on "hub" => {};
            disconnect_upstash: Confirm on "hub" => {};
            set_box_access: Confirm on "hub" => { "access": { "session": { "mode": "fresh" } } };
            extend_box: Write on "hub" => { "machine": "m1", "extra_usd": 2.0 };
            pair_code: Confirm on "hub" => {};
            revoke_invite: Confirm on "hub" => { "id": "i1" };
            stage: Write on "hub" => { "connector": "c1", "issue": "uuid-1", "stage": "coding" };
            slack_reply: Write on "hub" => { "connector": "c1", "channel": "C1", "thread": "1.2", "text": "On it." };
            linear_create: Write on "hub" => { "connector": "c1", "channel": "C1", "thread": "1.2", "title": "Fix login", "description": "From Slack." };
            linear_take: Write on "hub" => { "connector": "c1", "channel": "C1", "thread": "1.2", "issue": "ENG-7" };
            approve_agent: Confirm on "hub" => { "request": "r1", "scopes": ["sessions"], "projects": [{ "machine": "m1", "path": "/w/site" }] };
            deny_agent: Confirm on "hub" => { "request": "r1" };
            revoke_agent: Confirm on "hub" => { "id": "g1" };
            set_box_claude_token: Confirm on "hub" => { "name": "bloom" };
            remove_box_claude_token: Confirm on "hub" => { "name": "bloom" };
            mint_box_claude_token: Confirm on "hub" => { "machine": "m1", "account": "bloom" };
            cancel_box_claude_token: Write on "hub" => { "machine": "m1", "account": "bloom" };
        }

        #[test]
        fn open_box() {
            wired("open_box", Write, "hub", true, json!({ "repo": "acme/site", "budget_usd": 3.0 }));
            assert!(HUB_LIMITS.contains(&"open_box"), "the hub holds open_box to the box budget and cap");
            let Ok(Parsed::Hub(HubCall::OpenBox { budget_usd, .. })) = parse(find("open_box").unwrap(), &json!({ "repo": "acme/site", "budget_usd": 3.0 })) else { panic!("open_box") };
            assert_eq!(budget_usd, Some(3.0));
        }

        #[test]
        fn run_workflow() {
            wired("run_workflow", Write, "hub", true, json!({ "id": "c1" }));
            assert!(HUB_LIMITS.contains(&"run_workflow"), "the hub holds run_workflow to the caller's limits");
            let Ok(Parsed::Hub(HubCall::RunWorkflow { id })) = parse(find("run_workflow").unwrap(), &json!({ "id": "c1" })) else { panic!("run_workflow") };
            assert_eq!(id, "c1");
        }

        #[test]
        fn connect_upstash_without_its_blanks_does_not_parse() {
            let bare = parse(find("connect_upstash").unwrap(), &json!({})).err().unwrap_or_default();
            assert!(bare.starts_with("connect_upstash: missing field `key`"), "without the blanks the call does not parse: {bare}");
        }
    }

    /// One test per action P3c wires: the account's own MCP servers. An agent names a server's
    /// secrets and never sends their values, which the user types in the app at the approval.
    mod p3c {
        use super::p3b::wired;
        use super::*;

        const SERVER: &str = r#"{ "name": "ctx", "transport": { "type": "stdio", "command": "npx", "args": ["-y", "@acme/ctx"], "env": ["API_KEY"] } }"#;

        fn server() -> Value {
            serde_json::from_str(SERVER).unwrap()
        }

        #[test]
        fn list_mcp_servers() {
            wired("list_mcp_servers", Read, "hub", false, json!({}));
        }

        #[test]
        fn save_mcp_server() {
            wired("save_mcp_server", Confirm, "hub", false, json!({ "server": server() }));
            wired("save_mcp_server", Confirm, "hub", false, json!({ "id": "s1", "server": server() }));
        }

        #[test]
        fn enable_mcp_server() {
            wired("enable_mcp_server", Write, "hub", false, json!({ "id": "s1", "enabled": false }));
        }

        #[test]
        fn remove_mcp_server() {
            wired("remove_mcp_server", Confirm, "hub", false, json!({ "id": "s1" }));
        }

        #[test]
        fn import_mcp_server() {
            wired("import_mcp_server", Confirm, "hub", false, json!({ "machine": "m1", "account": "claude", "name": "linear", "project": "github.com/acme/site" }));
        }

        #[test]
        fn the_machines_load_and_the_apps_import_reader_are_closed_to_agents() {
            for name in ["load_mcp_servers", "mcp_importable"] {
                let action = find(name).unwrap();
                assert_eq!((action.class, action.wired), (Excluded, false), "{name}");
            }
        }

        #[test]
        fn a_value_an_agent_puts_under_secrets_is_refused_and_never_reaches_the_trail() {
            for sent in [json!({ "env": { "REGION": "eu-west-9" } }), json!({ "headers": { "X-Org": "eu-west-9" } }), json!("eu-west-9")] {
                let args = json!({ "server": server(), "secrets": sent });
                assert_eq!(secret_arg(&args, &[]).as_deref(), Some("secrets"), "{args}");
                assert!(!summary(&args).contains("eu-west-9"), "{}", summary(&args));
                assert_eq!(without_asked(find("save_mcp_server").unwrap(), &args), json!({ "server": server() }));
            }
            assert_eq!(secret_arg(&json!({ "server": server(), "secrets": { "env": {} } }), &[]), None, "no value, nothing sent");
            assert!(!redact(json!({ "secrets": { "env": { "API_KEY": "abc" } } }), &[]).to_string().contains("abc"), "an answer never shows a secrets object");
        }

        #[test]
        fn the_values_typed_at_the_approval_land_in_the_secrets_the_call_takes() {
            let typed = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
            let save = find("save_mcp_server").unwrap();
            let stored = json!({ "server": server() });
            let args = with_entered(save, &stored, &typed(&[("secrets.env.API_KEY", "ctx-key-1")])).unwrap();
            assert_eq!(args, json!({ "server": server(), "secrets": { "env": { "API_KEY": "ctx-key-1" } } }));
            let Ok(Parsed::Hub(HubCall::SaveMcpServer { secrets, .. })) = parse(save, &args) else { panic!("save_mcp_server") };
            assert_eq!(secrets.env.get("API_KEY").map(String::as_str), Some("ctx-key-1"));
            let both = with_entered(save, &stored, &typed(&[("secrets.env.A", "1"), ("secrets.headers.B", "2")])).unwrap();
            assert_eq!(both["secrets"], json!({ "env": { "A": "1" }, "headers": { "B": "2" } }));
            assert_eq!(with_entered(save, &stored, &typed(&[("server.name", "x")])), Err("save_mcp_server does not take server.name".into()), "the user's form changes only the secrets");
            assert_eq!(with_entered(save, &stored, &BTreeMap::new()), Ok(stored.clone()), "no secret typed: the server says which it needs");
            assert_eq!(with_blank_asks(save, &stored), stored, "an asked object is left to its default");
            assert!(hide_entered("bad ctx-key-1 here", &typed(&[("secrets.env.API_KEY", "ctx-key-1")])) == format!("bad {HIDDEN} here"));
        }
    }

    #[test]
    fn a_confirmation_keeps_no_asked_field_and_the_decision_takes_only_the_asked_ones() {
        let upstash = find("connect_upstash").unwrap();
        assert_eq!(without_asked(upstash, &json!({ "key": "", "claude_token": null, "note": "x" })), json!({ "note": "x" }));
        assert_eq!(with_blank_asks(upstash, &Value::Null), json!({ "key": "", "claude_token": "" }));
        assert_eq!(with_blank_asks(find("rename_machine").unwrap(), &Value::Null), Value::Null);
        let typed = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();

        let both = with_entered(upstash, &json!({}), &typed(&[("key", "box_k"), ("claude_token", "sk-ant-oat01-x")])).unwrap();
        assert_eq!(both, json!({ "key": "box_k", "claude_token": "sk-ant-oat01-x" }));
        assert!(matches!(parse(upstash, &both), Ok(Parsed::Hub(HubCall::ConnectUpstash { .. }))));
        assert_eq!(with_entered(upstash, &json!({}), &typed(&[("key", "box_k")])), Err("connect_upstash needs claude_token, which the user types in the app".into()));
        assert_eq!(with_entered(upstash, &json!({}), &typed(&[("key", " "), ("claude_token", "t")])), Err("connect_upstash needs key, which the user types in the app".into()));
        assert_eq!(with_entered(upstash, &json!({}), &typed(&[("key", "k"), ("claude_token", "t"), ("name", "x")])), Err("connect_upstash does not take name".into()));
        assert_eq!(with_entered(find("remove_machine").unwrap(), &json!({ "id": "m1" }), &typed(&[("key", "k")])), Err("remove_machine takes nothing typed in the app".into()));
        assert_eq!(with_entered(find("remove_machine").unwrap(), &json!({ "id": "m1" }), &BTreeMap::new()), Ok(json!({ "id": "m1" })));
        let account = json!({ "provider": "claude", "name": "Side" });
        assert_eq!(with_entered(find("add_account").unwrap(), &account, &BTreeMap::new()), Ok(account.clone()), "a login account needs no key");
        assert_eq!(with_entered(find("set_account_key").unwrap(), &json!({ "account": "go" }), &BTreeMap::new()), Err("set_account_key needs key, which the user types in the app".into()));
    }

    #[test]
    fn what_the_user_typed_never_shows_in_an_error_or_an_answer() {
        let typed = BTreeMap::from([("key".to_owned(), " box_plainkey ".to_owned())]);
        assert_eq!(hide_entered("Upstash refused box_plainkey for sk-ant-api03-abcdefghijklmnop1234", &typed), "Upstash refused [hidden] for [hidden]");
        assert_eq!(hide_entered_in(json!({ "hint": "…key", "echo": ["box_plainkey"], "n": 1 }), &typed), json!({ "hint": "…key", "echo": ["[hidden]"], "n": 1 }));
        assert_eq!(hide_entered("nothing typed", &BTreeMap::from([("key".to_owned(), String::new())])), "nothing typed");
    }

    #[test]
    fn actions_lists_wired_actions_with_their_argument_schema_and_no_cmd() {
        let all = listing(None);
        let names: Vec<&str> = all.iter().map(|l| l.name).collect();
        assert!(names.contains(&"tabs") && names.contains(&"remove_machine") && names.contains(&"agent_audit"));
        assert!(!names.contains(&"write_input") && !names.contains(&"open_taste_chat") && !names.contains(&"act"), "excluded or unwired: {names:?}");
        let rename = all.iter().find(|l| l.name == "rename_machine").unwrap();
        assert_eq!((rename.class, rename.on), (Write, "hub"));
        assert_eq!(rename.args["required"], json!(["id", "name"]));
        assert_eq!(rename.args["properties"].as_object().unwrap().keys().collect::<Vec<_>>(), ["id", "name"]);
        let save = all.iter().find(|l| l.name == "memory_save").unwrap();
        assert_eq!(save.on, "machine");
        assert!(save.args["properties"].get("cmd").is_none() && save.args["properties"].get("path").is_some(), "{}", save.args);
        let tabs = all.iter().find(|l| l.name == "tabs").unwrap();
        assert_eq!(tabs.args["properties"], json!({}));
        let audit = all.iter().find(|l| l.name == "agent_audit").unwrap();
        assert!(audit.args.to_string().find("$ref").is_none(), "refs are written out: {}", audit.args);
        let found: Vec<&str> = listing(Some("Rename MACHINE")).iter().map(|l| l.name).collect();
        assert_eq!(found, ["rename_machine"]);
    }

    #[test]
    fn act_args_parse_into_the_call_and_a_bad_one_names_the_action() {
        let Ok(Parsed::Hub(HubCall::RenameMachine { id, name })) = parse(find("rename_machine").unwrap(), &json!({ "id": "m1", "name": "Mini", "cmd": "remove_machine" })) else { panic!("rename_machine") };
        assert_eq!((id.as_str(), name.as_str()), ("m1", "Mini"));
        let Ok(Parsed::Node(Call::MemorySave { path, title, .. })) = parse(find("memory_save").unwrap(), &json!({ "path": "/w/site", "title": "Deploys", "text": "From tags." })) else { panic!("memory_save") };
        assert_eq!((path.as_str(), title.as_str()), ("/w/site", "Deploys"));
        assert!(matches!(parse(find("tabs").unwrap(), &Value::Null), Ok(Parsed::Node(Call::Tabs))));
        let Err(missing) = parse(find("rename_machine").unwrap(), &json!({ "id": "m1" })) else { panic!("no name") };
        assert!(missing.starts_with("rename_machine: missing field `name`"), "{missing}");
        let Err(shape) = parse(find("machines").unwrap(), &json!(["m1"])) else { panic!("an array") };
        assert_eq!(shape, "machines: args must be an object of the action's fields");
    }

    #[test]
    fn the_trail_notes_a_call_that_loosens_a_sessions_permissions() {
        let note = |action: &str, args: Value| widening(&parse(find(action).unwrap(), &args).unwrap());
        assert_eq!(note("set_bypass", json!({ "tab": "tab-1", "on": true, "cols": 80, "rows": 24 })).as_deref(), Some("loosened permissions: tab-1 now skips its permission prompts"));
        assert_eq!(note("open_tab", json!({ "path": "/w/site", "bypass": true })).as_deref(), Some("loosened permissions: the new tab in /w/site skips its permission prompts"));
        assert_eq!(note("set_bypass", json!({ "tab": "tab-1", "on": false, "cols": 80, "rows": 24 })), None);
        assert_eq!(note("open_tab", json!({ "path": null, "bypass": false })), None);
        assert_eq!(note("device_agents", json!({ "allow": true })).as_deref(), Some("loosened permissions: new sessions on this machine may drive its simulators and emulators"));
        assert_eq!(note("device_agents", json!({ "allow": false })), None);
        assert_eq!(note("device_agents", json!({})), None);
    }

    #[test]
    fn a_workflow_that_skips_permission_prompts_is_noted_and_one_that_asks_is_not() {
        let workflow = |mode: &str, step: &str| {
            json!({
                "connector": {
                    "name": "Fix", "enabled": true, "trigger": { "kind": "called" }, "folder": "/w/site", "model": "", "effort": "high", "permissionMode": mode, "worktree": false, "instructions": "",
                    "graph": { "nodes": [{ "id": "s", "name": "Fix", "position": [0, 0], "type": "session", "permission": step }], "edges": [] }
                }
            })
        };
        let parsed = |args: Value| parse(find("save_connector").unwrap(), &args).unwrap();
        assert!(!skips_prompts(&parsed(workflow("acceptEdits", "plan"))));
        assert!(skips_prompts(&parsed(workflow("bypassPermissions", ""))));
        assert!(skips_prompts(&parsed(workflow("manual", "bypassPermissions"))), "a step's own setting counts");
        assert_eq!(widening(&parsed(workflow("bypassPermissions", ""))).as_deref(), Some("loosened permissions: the saved workflow runs a session that skips its permission prompts"));
        assert_eq!(widening(&parsed(workflow("auto", ""))), None);
    }

    #[test]
    fn an_actions_check_refuses_arguments_that_parse() {
        fn no_blank_name(call: &Parsed) -> Result<(), String> {
            match call {
                Parsed::Hub(HubCall::RenameMachine { name, .. }) if name.trim().is_empty() => Err("rename_machine: name a machine with words".into()),
                _ => Ok(()),
            }
        }
        let checked = find("rename_machine").unwrap().checks(no_blank_name);
        assert_eq!(parse(&checked, &json!({ "id": "m1", "name": " " })).err(), Some("rename_machine: name a machine with words".into()));
        assert!(matches!(parse(&checked, &json!({ "id": "m1", "name": "Mini" })), Ok(Parsed::Hub(HubCall::RenameMachine { .. }))));
    }

    #[test]
    fn a_key_token_or_password_in_the_arguments_is_named_and_an_empty_one_is_not() {
        assert_eq!(secret_arg(&json!({ "key": "lin_api_123" }), &[]), Some("key".into()));
        assert_eq!(secret_arg(&json!({ "settings": { "github_token": "x" } }), &[]), Some("settings.github_token".into()));
        assert_eq!(secret_arg(&json!({ "items": [{ "password": "hunter2" }] }), &[]), Some("items.password".into()));
        assert_eq!(secret_arg(&json!({ "key": "", "token": null, "name": "Mini" }), &[]), None);
        assert_eq!(secret_arg(&json!({ "path": "/w/site", "title": "Deploys", "text": "the key is in the vault" }), &[]), None);
        let project = json!({ "project": { "key": "github.com/acme/site", "name": "site", "repo": "github.com/acme/site" } });
        assert_eq!(secret_arg(&project, &[]), Some("project.key".into()));
        assert_eq!(secret_arg(&project, &["project.key"]), None, "an action may name the paths that hold no secret");
        assert_eq!(secret_arg(&json!({ "project": { "key": "x" }, "token": "abc" }), &["project.key"]), Some("token".into()));
        assert_eq!(find("share_project").map(|a| a.sends), Some(&["project.key"][..]));
    }

    #[test]
    fn redact_hides_secret_fields_and_secret_shaped_text_and_keeps_a_projects_key() {
        let answer = json!({
            "key": "github.com/o/site",
            "token": "abc",
            "invite": { "id": "i1", "code": "K7QM-2XRP" },
            "text": "use sk-ant-api03-abcdefghijklmnop1234 then ghp_ABCDEFGHIJKLMNOP1234 or task-abcdefghijklmnopqrst",
            "jwt": "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "count": 3,
        });
        let shown = redact(answer, &["code"]);
        assert_eq!(shown["key"], "github.com/o/site");
        assert_eq!(shown["token"], "[hidden]");
        assert_eq!(shown["invite"], json!({ "id": "i1", "code": "[hidden]" }));
        assert_eq!(shown["text"], "use [hidden] then [hidden] or task-abcdefghijklmnopqrst");
        assert_eq!(shown["jwt"], "Bearer [hidden]");
        assert_eq!(shown["count"], 3);
        let pem = "a\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\n-----END OPENSSH PRIVATE KEY-----\nz";
        assert_eq!(mask_secrets(pem), "a\n[hidden]\nz");
        assert_eq!(mask_secrets("AKIAIOSFODNN7EXAMPLE and AKIA short"), "[hidden] and AKIA short");
        assert_eq!(mask_secrets("ação sk-ant-api03-abcdefghijklmnop1234!"), "ação [hidden]!");
    }

    #[test]
    fn a_convex_deploy_key_is_hidden_and_its_deployment_name_is_not() {
        let cases = [
            ("key dev:quick-lemur-789|eyJ2MiI6ImExYjIifQ== failed", "key [hidden] failed"),
            ("prod:happy-otter-123|eyJ2MiI6IjBhMWIyYzNkNGU1ZjYwNzE4MjkzYTRiNWM2ZDdlOGY5In0=, preview:calm-fox-456|eyJ2MiI6InByZXZpZXcifQ", "[hidden], [hidden]"),
            ("preview:acme:site|eyJ2MiI6InByZXZpZXcifQ", "[hidden]"),
            ("CONVEX_DEPLOY_KEY=happy-otter-123|eyJ2MiI6IjBhMWIyYzNkNGU1ZjYwNzE4MjkzYTRiNWM2ZDdlOGY5In0=", "CONVEX_DEPLOY_KEY=[hidden]"),
            ("adminKey anonymous-agent|01a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f809 ok", "adminKey [hidden] ok"),
            ("CONVEX_DEPLOYMENT=dev:quick-lemur-789", "CONVEX_DEPLOYMENT=dev:quick-lemur-789"),
            ("ls|grep foo", "ls|grep foo"),
            ("dev:quick-lemur-789 | tail", "dev:quick-lemur-789 | tail"),
        ];
        for (text, shown) in cases {
            assert_eq!(mask_secrets(text), shown, "{text}");
        }
    }

    #[test]
    fn a_pipe_in_ordinary_text_is_not_taken_for_a_convex_key() {
        let kept = [
            "sed 's|/usr/local/share|/opt|' config",
            "sed 's|/usr/local/share/pinned/a/very/long/path/of/more/than/forty/characters|/opt|'",
            "grep -E \"timeout|connection_refused_error\" hub.log",
            "| name | value |\n|------------------|------------------|\n| a | b |",
            "|--------------------------------------------|--------------------------------------------|",
            "npm run dev:api|tee build.log",
            "cat file|base64_decode_then_write_it_out_to_the_log_file_named",
            "happy-otter-123|short-token-of-20ch",
        ];
        for text in kept {
            assert_eq!(mask_secrets(text), text);
        }
    }

    #[test]
    fn the_audit_summary_hides_keys_and_cuts_long_text() {
        assert_eq!(summary(&json!({ "key": "lin_api_123", "name": "x" })), r#"{"key":"[hidden]","name":"x"}"#);
        let long = summary(&json!({ "text": "a".repeat(300) }));
        assert_eq!(long, format!(r#"{{"text":"{}…"}}"#, "a".repeat(200)));
        assert_eq!(summary(&Value::Null), "{}");
        assert!(summary(&json!({ "items": vec!["b".repeat(150); 20] })).chars().count() <= 1001);
    }

    #[test]
    fn an_act_and_its_answers_keep_their_wire_shape() {
        let act: AgentAct = serde_json::from_value(json!({ "tab": "tab-1a-3", "action": "machines" })).unwrap();
        assert_eq!(act, AgentAct { tab: "tab-1a-3".into(), action: "machines".into(), args: Value::Null, request_id: None, machine: None });
        assert_eq!(serde_json::to_value(ActAnswer::Done { result: json!([1]) }).unwrap(), json!({ "outcome": "done", "result": [1] }));
        assert_eq!(serde_json::to_value(ActAnswer::Pending { id: "c1".into(), next: PENDING_NEXT.into() }).unwrap()["outcome"], "pending");
        assert_eq!(serde_json::to_value(ActAnswer::Repeat { earlier: AuditOutcome::Done, at: 5 }).unwrap(), json!({ "outcome": "repeat", "earlier": "done", "at": 5 }));
        let confirmation = Confirmation { id: "c1".into(), at: 1, machine: "m1".into(), machine_name: "MacBook".into(), tab: "tab-1".into(), title: "Clean up".into(), role: ActorRole::Worker, action: "remove_machine".into(), about: String::new(), args: json!({ "id": "m2" }), target: None, request_id: None, decision: Some(Decision::Approved), decided_at: Some(2), outcome: Some("done".into()) };
        assert_eq!(notice_text(&confirmation, &Value::Null), "[pinned] The user approved your remove_machine request (c1): done.");
        let denied = Confirmation { decision: Some(Decision::Denied), outcome: None, ..confirmation };
        assert_eq!(notice_text(&denied, &Value::Null), "[pinned] The user denied your remove_machine request (c1). It did not run.");
    }
}
