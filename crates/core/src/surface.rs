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
    /// Refuses arguments that parse but an agent may not send. Its error is the refusal.
    pub check: Option<fn(&Parsed) -> Result<(), String>>,
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
    Action { name: cmd, side: Side::Node, cmd, class, wired: false, starts_work: false, about, hides: &[], check: None }
}

const fn h(cmd: &'static str, class: Class, about: &'static str) -> Action {
    Action { name: cmd, side: Side::Hub, cmd, class, wired: false, starts_work: false, about, hides: &[], check: None }
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
    n("open_scratch", Write, "Opens a new session tab in the folder for threads without a project, not started, and answers its id."),
    n("auto_resume", Excluded, "a thread's own switch for continuing after a usage limit, which the user sets"),
    n("start_claude", Write, "Starts the session in tab and answers its folder. It becomes your worker: it counts against its project's workers-at-once limit, is held to the time limit and budget, its first finished turn ends it as a task, and you get its notice. cols and rows size its terminal; 120 and 32 suit.").wired().starts_work(),
    n("set_bypass", Write, "Restarts a session with permissions skipped (on: true) or asked (on: false), resuming its conversation."),
    n("set_model", Write, "Restarts a session's conversation on model, optionally moving it to agent account first."),
    n("restart_claude", Write, "Restarts a session on its conversation, so its skills, plugins and MCP servers load again."),
    n("write_input", Excluded, TERMINAL),
    n("resize_pty", Excluded, TERMINAL),
    n("close_tab", Excluded, "closing a thread loses its place in the sidebar: agents archive it with archive_session, which the user can undo, or ask to delete it with delete_session"),
    n("set_folder", Write, "Moves a tab that has not started to another folder."),
    n("list_projects", Read, "The machine's projects, most recent first: folder, name, repo and the agent account its sessions run under.").wired(),
    n("project_history", Read, "The saved turns of the project at path, newest first, only one Claude session's when session is given.").wired(),
    n("git_remote", Excluded, GIT),
    n("browse", Excluded, "the folder picker's typeahead: list folders in your own shell"),
    n("create_project", Write, "Makes a new git repository named after name in ~/pinned, with a first commit, and opens it as a project."),
    n("publish_project", Write, "Pushes the project at path to a new private GitHub repository and answers its URL."),
    n("clone", Excluded, "it takes a GitHub token: the hub's clone action adds the account's own").named("node_clone"),
    n("make_folder", Write, "Makes the folder path with its parents and answers it, canonical."),
    n("search_threads", Read, "The sessions on the machine whose conversations hold query, prompt matches first.").wired(),
    n("run", Excluded, "the hub's way to start a workflow's session: its raw arguments and environment could pass for a workflow's"),
    n("deliver", Excluded, "the hub's way into a workflow's session, matched by environment"),
    n("upload", Excluded, "the app's file upload: you have your own files"),
    n("read_upload", Excluded, "the app's picture previews"),
    n("read_image", Excluded, "the app's picture previews"),
    n("learn_model_menu", Excluded, "the app's reading of claude's /model menu off a tab's screen"),
    n("list_accounts", Read, "The agent accounts on the machine (Claude Code, Codex, Cursor, OpenCode Go), each with whether it is signed in.").wired(),
    n("add_account", Confirm, "Adds an agent account on the machine. The user enters any API key in the app."),
    n("set_account_key", Confirm, "Replaces an agent account's API key. The user enters it in the app."),
    n("remove_account", Confirm, "Removes an agent account from the machine; its projects fall back to the default account."),
    n("set_project_account", Write, "Runs the project at path under another agent account, or the default one with null."),
    n("open_orchestrator", Write, "Opens the Captain tab of the project at path, not started, and answers its id."),
    n("enqueue", Write, "Hands text to the Captain of the project at path: typed when it is idle, its first prompt when stopped, else queued.").starts_work(),
    n("orchestrator_report", Excluded, "a Captain's report to the global orchestrator: a Captain calls tell_orchestrator"),
    n("open_global_orchestrator", Write, "Opens this machine's global orchestrator tab, not started, and answers its id."),
    n("open_taste_chat", Write, "Opens a thread that adjusts a scope's way of working by talking, not started, and answers its id."),
    n("global_orchestrator_settings", Read, "The machine's global orchestrator settings: its name, whether this machine hosts it, and its busy and report policies.").wired(),
    n("set_global_orchestrator_settings", Write, "Saves the machine's global orchestrator settings."),
    n("read_session", Read, "A session's state and its last items (20, at most 50) of conversation, read from the end of its transcript, on the session's machine or the one machine names.").named("node_read_session").wired(),
    n("memory_search", Read, "The memories of the project at path that best match query, best first, or the latest for an empty query. Any project of the account: pass machine for one on another machine.").wired(),
    n("memory_save", Write, "Saves a memory in the project at path, any project of the account (pass machine for another machine). With id, replaces that memory. Refused when the project has one with the same title or nearly the same meaning.").wired(),
    n("memory_forget", Write, "Removes the memory id of the project at path, any project of the account.").wired(),
    n("memory_status", Read, "How many memories the project at path has and whether search also matches by meaning.").wired(),
    n("sign_in", Confirm, "Opens the CLI's login for an agent account in a new tab, where the user signs in."),
    n("switch_account", Write, "Moves a session's conversation to another agent account signed in on the machine and resumes it there."),
    n("thread", Read, "A session's conversation items from byte from of its transcript (0: the start) and the offset to read next. node_read_session reads its end.").wired(),
    n("prompt", Write, "Types text into a session as one prompt and submits it."),
    n("workspace", Excluded, GIT),
    n("pull_request", Excluded, GIT),
    n("pr_status", Excluded, GIT),
    n("favicon", Excluded, "the sidebar's project icons"),
    n("regenerate_title", Write, "Writes a new title for a session's conversation and answers it."),
    n("pr_list", Excluded, GIT),
    n("pr_action", Excluded, GIT),
    n("diff", Excluded, GIT),
    n("ports", Excluded, "the app's browser panel: list ports in your own shell"),
    n("editors", Excluded, "opens the user's editors on their screen"),
    n("open_in", Excluded, "opens the user's editors on their screen"),
    n("commands", Excluded, "the composer's slash-command list"),
    n("files", Excluded, FILES),
    n("skills", Read, "Every skill folder on the machine: personal, project and plugin skills, and where each copy lives.").wired(),
    n("read_skill", Read, "The text of file (SKILL.md when left out) in the skill folder dir that skills lists.").wired(),
    n("save_skill", Write, "Writes a skill file to each listed copy unchanged since its read, then reloads the sessions that load it."),
    n("branches", Excluded, GIT),
    n("checkout", Excluded, GIT),
    n("pull_requests", Excluded, GIT),
    n("pull_request_stack", Excluded, GIT),
    n("pull_request_filter_options", Excluded, GIT),
    n("pull_request_files", Excluded, GIT),
    n("token_usage", Read, "The tokens and cost the machine's Claude transcripts record from unix time since on, by model and day.").wired(),
    n("usage_prices", Read, "The model prices and mappings the user set on the machine.").wired(),
    n("set_usage_prices", Write, "Sets or clears model prices and mappings on the machine."),
    n("usage_limits", Read, "The account and plan limits Claude's own usage check reports.").wired(),
    n("page_meta", Excluded, "the app's link previews"),
    n("refs", Excluded, GIT),
    n("git", Excluded, GIT),
    n("project_scripts", Read, "The scripts of a session's project, which its header runs.").wired(),
    n("save_project_scripts", Write, "Replaces the scripts of a session's project."),
    n("checkout_pr", Excluded, GIT),
    n("set_workspace", Write, "Moves a session that has not begun to a worktree of its project, or back to the project's checkout, and restarts it there."),
    n("read_file", Excluded, FILES),
    n("watch_pr", Write, "Starts or stops watching a pull request for a session, which wakes it when the checks finish, someone comments, or its branch conflicts."),
    n("file_url", Excluded, FILES),
    n("write_file", Excluded, FILES),
    n("search", Excluded, FILES),
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
    n("device_hub", Write, "Starts the machine's device hub, which mirrors its simulators and emulators, and answers its address."),
    n("device_agents", Write, "Whether new sessions may drive the machine's simulators and emulators, after setting it to allow when given."),
    n("device", Write, "Runs op against the machine's simulators and emulators."),
    n("subagent", Read, "The conversation of the subagent a session's Agent tool call tool started.").wired(),
    n("rewind", Write, "Restarts a session on a copy of its conversation that stops before the prompt before, optionally putting its files back as they were."),
    n("fork", Write, "Opens a new tab, not started, that resumes a copy of a session's conversation through the message through."),
    n("save_file", Excluded, FILES),
    n("open_task", Write, "Opens a tab, not started, that resumes a delegated task's conversation."),
    n("cancel_task", Write, "Stops a delegated task that the session in tab owns."),
    n("deliver_task", Excluded, "the app's send-now for a worker's notice: task_status reads it"),
    n("watch_threads", Excluded, "the app's live transcript updates"),
    n("update_self", Confirm, "Updates pinned on the machine to the latest release and restarts it, which stops the sessions it runs."),
    n("sync_stopped", Read, "Why the machine stopped syncing projects and plugins, or null while it syncs.").wired(),
    n("reset_sync", Write, "Sets an unreadable sync state aside, so the machine's next sync starts over as a new machine's."),
    n("delegate", Excluded, "the hub's way to start a worker here for another machine: call delegate_task with target.machine").named("node_delegate"),
    n("remote_task", Excluded, "the hub's way to read a worker running here for another machine: call task_status").named("node_remote_task"),
    n("remote_ended", Excluded, "the hub's way to tell a machine its remote worker ended").named("node_remote_ended"),
    n("act_for", Excluded, "the hub's way to run an agent's act on its machine"),
    n("notice", Excluded, "the hub's way to tell a session what the user decided"),
    // The account, on the hub.
    h("get_settings", Read, "The account's integrations (who is connected to Linear, Slack, GitHub, Vercel and Upstash, never a key), its workflows, their last check, and box access.").wired(),
    h("connect_linear", Confirm, "Connects Linear. The user enters the key in the app; an agent never sends one."),
    h("disconnect_linear", Confirm, "Disconnects Linear."),
    h("linear_suggestions", Read, "Linear's labels, columns and teams, for a workflow's form.").wired(),
    h("connect_slack", Confirm, "Connects Slack. The user enters the token in the app; an agent never sends one."),
    h("disconnect_slack", Confirm, "Disconnects Slack."),
    h("slack_suggestions", Read, "The Slack channel names a workflow may watch.").wired(),
    h("connect_github", Confirm, "Connects GitHub. The user enters the token in the app; an agent never sends one."),
    h("disconnect_github", Confirm, "Disconnects GitHub."),
    h("github_repos", Read, "The GitHub repositories the account's token reaches, most recently pushed first.").wired(),
    h("connect_vercel", Confirm, "Connects Vercel's AI gateway. The user enters the key in the app; an agent never sends one."),
    h("disconnect_vercel", Confirm, "Disconnects Vercel."),
    h("vercel_models", Read, "The chat and decision models a workflow may use.").wired(),
    h("connect_upstash", Confirm, "Connects Upstash for boxes. The user enters the keys in the app; an agent never sends one."),
    h("disconnect_upstash", Confirm, "Deletes the account's boxes and disconnects Upstash."),
    h("set_box_access", Confirm, "Sets what the account's boxes may reach."),
    h("answer_box", Confirm, "Allows or refuses a box's pending permission question."),
    h("clone", Write, "Clones repo on machine with the account's GitHub token, or finds the clone it has, and answers the folder."),
    h("shared_projects", Read, "The account's synced projects, most recently shared first.").wired(),
    h("share_project", Write, "Offers a project to the account's other machines to clone."),
    h("sync_list", Excluded, SYNC),
    h("sync_push", Excluded, SYNC),
    h("sync_pull", Excluded, SYNC),
    h("sync_mark_executable", Excluded, SYNC),
    h("save_connector", Write, "Saves a workflow and answers it as stored."),
    h("delete_connector", Confirm, "Deletes a workflow."),
    h("generate_workflow", Write, "Drafts a workflow map from a plain description with a chat model."),
    h("open_box", Write, "Starts an interactive session on a new Upstash Box cloning repo.").starts_work(),
    h("box_status", Read, "The phase, age and budget of the box behind machine.").wired(),
    h("extend_box", Confirm, "Adds extra_usd to the budget of the box behind machine."),
    h("machines", Read, "Every machine of the account: id, name, OS, whether it is online, when it was last seen, its pinned version and update, and its box.").wired(),
    h("rename_machine", Write, "Renames the account's machine id to name.").wired(),
    h("remove_machine", Confirm, "Removes a machine from the account: its sessions stop showing, and a box's machine is deleted with its box.").wired(),
    h("pair_code", Confirm, "Makes a one-time code, and the command that uses it, to pair a new machine with the account. You get its expiry; the user gets the code.").hides(&["code", "command"]),
    h("create_invite", Confirm, "Makes a single-use invite code that lets one person create an account on this hub. You get its id and expiry; the user gets the code.").wired().hides(&["code"]),
    h("invites", Read, "The hub's signup mode and the account's unused invite codes, named by their first characters.").wired(),
    h("revoke_invite", Confirm, "Stops an unused invite code from working."),
    h("import", Excluded, "a signed-out app's one-time move of its connectors"),
    h("stage", Write, "Moves a workflow's issue to the stage's Linear column."),
    h("slack_reply", Write, "Replies in a workflow's Slack thread."),
    h("linear_create", Write, "Files a Linear issue for a workflow's Slack thread."),
    h("linear_take", Write, "Ties an existing Linear issue to a workflow's Slack thread."),
    h("external_agents", Read, "The hub's MCP endpoint and the external agents the account let in, with their scopes and projects.").wired(),
    h("agent_request", Excluded, "the consent page's own read: external_agents lists access"),
    h("approve_agent", Confirm, "Lets a pending external agent in with scopes on projects."),
    h("deny_agent", Confirm, "Refuses a pending external agent."),
    h("revoke_agent", Confirm, "Ends an external agent's access at once."),
    h("relay", Excluded, "the global orchestrator's bridge between machines: it calls send_to_captain"),
    h("overview", Read, "Every project of the account on every machine with its Captain's state, what waits on the user, every machine, and every session.").wired(),
    h("read_captain", Read, "The latest replies of the Captain of the project at path on machine.").wired(),
    h("get_tastes", Read, "The account's way of working: the orchestrator's, the Captains' and each project's, with the delegation limits.").wired(),
    h("set_taste", Confirm, "Changes fields of one scope of the account's way of working."),
    h("reset_taste_field", Confirm, "Makes a field of one scope of the way of working inherit again."),
    h("import_tastes", Excluded, "a signed-out app's one-time move of its way of working"),
    h("read_session", Read, "Any session's state and last items, found on the account's online machines by its tab id, on machine when given.").wired(),
    h("record_tasks", Excluded, "machines send their delegated tasks' records by themselves"),
    h("delegation_stats", Read, "How the account's delegated tasks went over the last days (1, 7 or 30), per model.").wired(),
    h("delegate", Excluded, "a machine's way to run delegate_task on another machine or a box: call delegate_task"),
    h("remote_task", Excluded, "a machine's way to read its remote worker: call task_status"),
    h("remote_ended", Excluded, "a machine's way to report a remote worker's end"),
    h("set_box_claude_token", Confirm, "Keeps a Claude token for boxes that run as a pinned account. The user enters the token in the app; an agent never sends one."),
    h("remove_box_claude_token", Confirm, "Forgets the Claude token kept for boxes that run as a pinned account."),
    h("act", Excluded, "the surface itself: call act"),
    h("confirmations", Read, "The actions waiting on the user's approval, then the last 20 decided, with what each came to.").wired(),
    h("decide_confirmation", Excluded, "only the user decides, in the app"),
    h("agent_audit", Read, "The audit trail of agents' acts on this account, newest first: limit rows (50, at most 200) from unix second before and earlier.").wired(),
];

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

fn secret_name(name: &str, sending: bool) -> bool {
    let name = name.to_ascii_lowercase();
    ["token", "password", "secret", "api_key", "claude_token"].contains(&name.as_str())
        || ["_token", "_secret", "_password"].iter().any(|end| name.ends_with(end))
        || (sending && (name == "key" || name.ends_with("_key")))
}

/// The first field of `args`, at any depth, that holds a password, key or token, which an agent
/// never sends.
pub fn secret_arg(args: &Value) -> Option<String> {
    match args {
        Value::Object(fields) => fields.iter().find_map(|(name, value)| match value {
            Value::String(text) if secret_name(name, true) && !text.trim().is_empty() => Some(name.clone()),
            value => secret_arg(value).map(|inner| format!("{name}.{inner}")),
        }),
        Value::Array(items) => items.iter().find_map(secret_arg),
        _ => None,
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
            if let Some(len) = pem_block(rest).or_else(|| jwt(rest)).or_else(|| {
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
    fn excluded_never_runs_every_read_runs_and_hub_work_waits_for_a_limits_path() {
        for action in ACTIONS {
            assert!(!(action.class == Excluded && (action.wired || action.starts_work)), "{} is excluded", action.name);
            assert!(action.class != Read || action.wired, "{} is a read that is not wired", action.name);
            assert!(!(action.side == Side::Hub && action.starts_work && action.wired), "{} starts work on the hub, where nothing holds it to the limits yet", action.name);
            assert!(!action.about.is_empty(), "{} says nothing", action.name);
        }
        let wired: Vec<&str> = ACTIONS.iter().filter(|a| a.wired && a.class != Read).map(|a| a.name).collect();
        assert_eq!(wired, ["open_tab", "start_claude", "memory_save", "memory_forget", "rename_machine", "remove_machine", "create_invite"]);
    }

    #[test]
    fn actions_lists_wired_actions_with_their_argument_schema_and_no_cmd() {
        let all = listing(None);
        let names: Vec<&str> = all.iter().map(|l| l.name).collect();
        assert!(names.contains(&"tabs") && names.contains(&"remove_machine") && names.contains(&"agent_audit"));
        assert!(!names.contains(&"write_input") && !names.contains(&"prompt") && !names.contains(&"act"), "excluded or unwired: {names:?}");
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
        assert_eq!(secret_arg(&json!({ "key": "lin_api_123" })), Some("key".into()));
        assert_eq!(secret_arg(&json!({ "settings": { "github_token": "x" } })), Some("settings.github_token".into()));
        assert_eq!(secret_arg(&json!({ "items": [{ "password": "hunter2" }] })), Some("items.password".into()));
        assert_eq!(secret_arg(&json!({ "key": "", "token": null, "name": "Mini" })), None);
        assert_eq!(secret_arg(&json!({ "path": "/w/site", "title": "Deploys", "text": "the key is in the vault" })), None);
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
