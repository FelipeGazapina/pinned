# Global orchestrator contracts

Defined by the foundation unit, branch `global-orch/foundation` (PR link at the end). Every later unit starts from that branch. A unit that finds a contract wrong says so in its report and does not change it silently.

Paths are relative to the repo root. Line numbers are as of the foundation PR head and drift; search by name.

## Contents

1. Identity of the two roles
2. Settings
3. Node calls (`Call`)
4. Hub calls (`HubCall`)
5. Reply and payload types
6. Session tools
7. The delivery primitive and the ledger
8. Text formats
9. Seams and the unit that fills each
10. Files each later unit mainly touches
11. Rules every unit keeps
12. Known gaps
13. Agent capability surface (0.17.22)

## 1. Identity of the two roles

| Role | How a tab is marked | Where it runs | `TabView` |
|---|---|---|---|
| Global orchestrator | env `PINNED_ROLE=global-orchestrator` (`global_orchestrator::ROLE`, `global_orchestrator::env()`, `is_global(env)`) | `~/.pinned/orquestrador` (`global_orchestrator::FOLDER` under home). Made on open. Never a listed project. | `orchestrator_global: true`, `role: null` |
| Capitão | env `PINNED_ROLE=orchestrator` + `PINNED_PROJECT=<path>` (existing `Role::Orchestrator`) | the project folder | `role: "orchestrator"` |

- There is no new `Role` variant. An older app shows the global tab as a plain thread.
- One global tab per machine: `open_global_orchestrator` returns the lowest-numbered tab with the global env, else adds one, not started.
- The global folder's memory store is the global memory. `project_of_tab(global tab)` answers the folder, so `memory_*` tools and automatic recall use it. `known_project(folder)` accepts it, so `Call::MemorySearch/Save/Forget/Status { path: <folder> }` work. `Call::ListProjects` never lists it.
- In tests the folder is `<config dir>/home/.pinned/orquestrador` (`test_binary()`), so tests never touch the real home.
- `launch_tab` appends `global::prompt(<name>, <hostname>)` to the global tab's `--append-system-prompt`, after the device agent prompt. `role_of` is `None` for it, so it gets no role prompt and no disallowed tools.

## 2. Settings

`global_orchestrator::Settings`, schema name `GlobalOrchestratorSettings`. One file per machine, `<config dir>/global-orchestrator.json`, like `orchestration.json`.

```json
{ "name": "Orchestrator", "host": false, "busy": "queue", "reports": "auto", "group_reports": true, "read_aloud": false }
```

| Field | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | `"Orchestrator"` | Chat name and message prefix. UI options: Orchestrator, Admiral, Commander. `cleaned()` trims it, drops `[` `]` and control characters, caps it at 40 characters, and puts the default back when blank. |
| `host` | bool | `false` | This machine hosts the account's global orchestrator. One machine per account. The UI-settings unit keeps it unique across machines. |
| `busy` | `"queue"` \| `"refuse"` | `"queue"` | `BusyPolicy`. Refuse answers `ocupado` instead of queueing. |
| `reports` | `"auto"` \| `"tool_only"` | `"auto"` | `ReportPolicy`. Auto reports at the end of every turn that an orchestrator message started, plus `tell_orchestrator`. |
| `group_reports` | bool | `true` | The chat folds consecutive reports into one row. UI only. |
| `read_aloud` | bool | `false` | The phone reads summaries aloud. UI only. |

Every field has a serde default, so a partial object reads. `Machine::global_settings()` reads and `Machine::set_global_settings(s)` cleans, saves and returns.

## 3. Node calls (`Call`, `crates/core/src/wire.rs`)

| Call | JSON | Reply |
|---|---|---|
| `Enqueue` | `{"cmd":"enqueue","path":"/w/site","text":"Ship it.","request_id":"r-1","sender":"Orquestrador","busy":"refuse"}`. `sender` defaults to `""` (this machine's settings name). `busy` is optional (this machine's setting). | `Handoff`. Errors: `request_id is required`, `the message is empty`, `request_id <id> was already used for another message`, `<path> is not a folder on this machine`, `<path> is the global orchestrator's folder, which has no Capitão`, `account <id> runs <provider>, which can't be a project's orchestrator`. |
| `OrchestratorReport` | `{"cmd":"orchestrator_report","request_id":"report-1","report":{CaptainReport}}` | `Handoff` with the global tab's id. Never `ocupado`. |
| `OpenGlobalOrchestrator` | `{"cmd":"open_global_orchestrator"}` | the tab id (string). Not started. |
| `GlobalOrchestratorSettings` | `{"cmd":"global_orchestrator_settings"}` | `Settings` |
| `SetGlobalOrchestratorSettings` | `{"cmd":"set_global_orchestrator_settings","settings":{Settings}}` | the `Settings` saved |
| `ReadSession` | `{"cmd":"read_session","tab":"tab-2b-7","items":5}`. `items` is optional: 20, at most 50. | `reading::read` of the tab: its state, `next`, title, question, and its last `items` conversation items, read from the end of its transcript (`reading::end_of`), so a transcript past 8 MiB returns its true last reply. Error: `no session <tab> on this machine`. |

- None of them but `ReadSession` is `may_wait`. They run on the machine's in-order call queue; `ReadSession` reads the disk off it, as `Thread` does.
- An older node answers any of them with `this machine runs pinned <v>, which can't answer that: unknown variant ...`. `pinned_core::link::is_unknown_command(err)` is true for it. The orchestrator tells Felipe that machine needs an update.

## 4. Hub calls (`HubCall`, `crates/core/src/wire.rs`)

| Call | JSON | Reply |
|---|---|---|
| `Relay` | `{"cmd":"relay","request_id":"r-1","to":{RelayTo}}` | `Handoff` from the target machine |
| `Overview` | `{"cmd":"overview"}` | `Overview` |
| `ReadSession` | `{"cmd":"read_session","tab":"tab-2b-7","machine":"mini","items":5}`. `machine` (an id or a name) and `items` are optional. | the target node's `Call::ReadSession` answer with `id`, `machine` and `machine_name` set, or `{"state":"offline",...}` for a named offline machine. Errors: `this account has no machine <m>`, `<name> has no session <tab>`, `no online machine of this account has a session <tab>`, `<a> and <b> each have a session <tab>: pass machine`. |
| `ReadCaptain` | `{"cmd":"read_captain","machine":"<machine id>","path":"/w/site"}` | what the node's own `read_captain` answers: `reading::read` of the Capitão with `project` set to `path`, or `{"state":"not_started",...}` when the project has no Capitão that runs or has a conversation, or `{"state":"offline",...}`. A machine id the account does not have is an error, `this account has no machine <id>` |

| `RecordTasks` | `{"cmd":"record_tasks","records":[TaskRecord]}`, at most 500 | `null`. Only from a machine: a client gets `only a machine records the tasks it ran`. The hub stamps `machine` and `machine_name` from the socket and upserts by (account, machine, task id). |
| `DelegationStats` | `{"cmd":"delegation_stats","days":7}` (1, 7 or 30) | `delegate::DelegationStats` of the account's records that ended in that period, from a client or a machine |

`RelayTo` is tagged by `kind`:

```json
{"kind":"captain","machine":"<machine id>","path":"/w/site","text":"Ship it.","sender":"Orquestrador","busy":"queue"}
{"kind":"orchestrator","report":{CaptainReport}}
{"kind":"captain_box","repo":"github.com/acme/site","size":"medium","text":"Ship it.","sender":"Orquestrador","busy":"queue"}
```

- `captain_box` direction: the hub takes it only from the host, as `captain`. It opens the Capitão of `repo` in an Upstash Box with the account's own connection (`Boxes::open_captain`): the Captains box permission (`BoxAccess.orchestrator`) decides (Off and One box, reused refuse, Ask files a question on the Upstash card), the account's box cap applies, and the row is `connector = "captain"`, subject `the Captain of <clone url>`. It answers at once `{"outcome":"iniciado","machine":"<box machine id>","machineName":"Box · Captain <name>","box":"<row id>"}`, boots the box in the background, then calls `Call::Clone { repo, token, dest: null }` and `Call::Enqueue` there. While that box runs, a later `captain_box` for the same repo goes to it and answers `{"outcome":"entregue","tab":...}` with the box fields; while it boots it is refused, `this project's box is still booting`. The box goes once its tabs stay idle for the box policy's idle limit.

`crates/server/src/relay.rs` `serve` answers all three. `http.rs` `machine_frame` passes the sending machine as the source, and `hub.rs` `hub_call` passes a signed-in client. A client may call any machine of the account anyway, so the source checks below apply only to a machine.

- `captain` direction: the hub takes it only from the machine that hosts the account's global orchestrator, then calls `Call::Enqueue { path, text, request_id, sender, busy }` on `machine` under `Hub::hold_captain`.
- `orchestrator` direction: the hub takes it only from a machine whose tab `report.tab` is a Capitão (`role == orchestrator`) with `cwd == report.path`. It fills `report.machine` and `report.machine_name` from the sending socket, finds the host machine, and calls `Call::OrchestratorReport { request_id, report }` there.
- `Overview`, `ReadCaptain` and `ReadSession`: the hub takes them only from the host, as `captain` relays. A refused machine reads `<name> does not host the global orchestrator (<host> does), so it can't list the account's projects` (or `read a Capitão`, `read a session`, or `send to a Capitão`).
- `ReadSession` finds the tab in the tabs each online machine last reported (`Call::Tabs` for one that has reported none), then calls `Call::ReadSession` there. A node too old for it is read with `Call::Thread { from: 0 }`, as `ReadCaptain` reads.
- The host is the one online machine whose `Call::GlobalOrchestratorSettings` answers `host: true`, asked each time and never cached. No host, two hosts and a silent machine each get their own error.
- `Overview` builds one `MachineSnapshot` per machine the account has, in the order they joined, and returns `global_orchestrator::overview` of them:
  - Tabs are what each machine last reported (`MachineEvent::Tabs`). The hub asks `Call::Tabs` only of a machine that has reported none yet.
  - Each online machine is asked `Call::ListProjects` and `Call::GlobalOrchestratorSettings` at once, all machines in parallel, waiting at most 5 s (`LOOK_WAIT`). The asking machine's own settings wait 10 s (`HOST_WAIT`), as in the host check, because they decide whether it may ask.
  - A machine that refuses any of its calls with an unknown command is listed with `problem: "precisa atualizar o pinned"`, and its projects still list when `ListProjects` answered. A machine whose `ListProjects` or `Call::Tabs` does not answer in time gets `problem: "não respondeu"`, and another refusal of those its text. A missing settings answer alone is no problem. None of these fails the call.
  - The hub keeps each machine's last project list in memory (`Account::listed`). A machine that does not answer, and an offline machine, show those projects, an offline one with `online: false` and `captain: "nenhum"`.
- `ReadCaptain` reads the tabs as `Overview` does, finds the Capitão with `captain_tab`, and reads its conversation with `Call::ReadSession` (`reading::DEFAULT_ITEMS` items, from the end of its transcript), or with `Call::Thread { from: 0 }` on a node too old for that, so a machine too old for the relay can still be read.
- An older hub answers any of them with `this hub runs pinned <v>, which can't answer that: unknown variant ...`, and `is_unknown_command` is true for it. The node falls back to this machine alone for `projects` and `waiting`, and answers `o hub ou a máquina precisa atualizar o pinned` for `send_to_captain` and `read_captain` (`needs_update`). Test: `a_node_reads_an_older_hubs_refusal_of_a_later_call_as_an_unknown_command`.
- `Link::call_hub` waits at most 60 s. The session socket waits 70 s for `projects`, `send_to_captain`, `read_captain`, `waiting` and `tell_orchestrator` (`reminders.rs` `send`), 10 s for other tools.

## 5. Reply and payload types (`crates/core/src/global_orchestrator.rs`)

All are wire-contract outbound roots except `Sent` and `Ledger`, which stay on the machine.

### `Handoff`

Tagged by `outcome`. Every variant carries the tab it went to.

| Variant | JSON |
|---|---|
| `Delivered` | `{"outcome":"entregue","tab":"tab-1a-3"}` |
| `Queued` | `{"outcome":"na_fila","tab":"tab-1a-3","position":2}`. `position` counts this message among those queued for the tab. |
| `Started` | `{"outcome":"iniciado","tab":"tab-1a-3"}` |
| `Busy` | `{"outcome":"ocupado","tab":"tab-1a-3"}`. Nothing typed or queued. Not remembered, so a retry with the same `request_id` may deliver. |

### `CaptainReport`

```json
{"machine":"<id or empty>","machine_name":"MacBook","project":"site","path":"/w/site","tab":"tab-1a-3","text":"...","answers":["r-1"]}
```

`answers` lists the `request_id`s whose turn the report ends. It is empty, and omitted, for `tell_orchestrator`. `line()` builds the text typed into the global tab (section 8).

### `Overview`

```json
{
  "projects": [{
    "key": "github.com/o/site",
    "name": "site",
    "repo": "github.com/o/site",
    "checkouts": [{
      "machine": "<id>", "machine_name": "Mac mini", "online": true, "version": "0.17.19",
      "path": "/Users/f/site", "captain": "livre", "captain_tab": "tab-1a-3", "workers": 2
    }]
  }],
  "waiting": [{
    "machine": "<id>", "machine_name": "MacBook", "tab": "tab-2b-7", "kind": "needs_input",
    "project": "/Users/f/site", "title": "Fix the form", "message": "Claude needs your permission to use Bash", "since": 1794234244
  }],
  "machines": [
    { "id": "<id>", "name": "Mac mini", "online": true, "version": "0.17.19" },
    { "id": "<id>", "name": "iMac", "online": true, "version": "0.17.10", "problem": "precisa atualizar o pinned" }
  ],
  "sessions": [{Session}]
}
```

- `key` is `repo` when the project has an origin, else `<machine id>:<path>`. Tools take it as `project`.
- `captain` is `CaptainState`: `"livre"` (idle, done, failed or stopped: a message types now or starts it; also needs-input with `interrupted` false, which is Claude Code's idle notification after a finished turn), `"trabalhando"` (running with status working, or with `TabView.orchestrator_turn`, or needs-input with `interrupted` true, a question mid-turn: a message queues), `"nenhum"` (no Capitão tab, or the machine is offline). `CaptainState::of(Option<&TabView>)` computes it. Use it on the hub and the node alike.
- `TabView.orchestrator_turn` (`#[serde(default)]`, omitted when false) is true on a Capitão while its ledger holds a message `in_flight` for its project (`Ledger::turns`). It covers the window between `iniciado` and the Capitão's first hook, when the tab runs with no status and a second message answers `na_fila`. The node sends its tabs again after each ledger change that starts such a turn (`enqueue_to_captain` on `entregue` or `iniciado`, `mark_typed`). `record_event` sends them after the turn-end answer clears it.
- `machines` (`#[serde(default)]`) lists every machine of the account, with or without projects. `problem` says why its projects may be missing or old.
- A node that is not linked, or whose hub is too old, answers `projects` with itself alone, under its hub id and hub name once the link's hello gave them (`Machine::set_hub_id`), else its hostname.
- `WaitingKind` is `"needs_input"` or `"failed"`. Adding a kind, such as a ready PR, is an additive enum change.
- `global_orchestrator::waiting_kind(&TabView)` decides whether a tab waits on the user, on the node and the hub alike. The sidebar's `waitingKindOf(view)` in `ui/global-chat.js` follows the same cases and returns the same strings. `crates/core/src/waiting_cases.json` lists the cases, and the Rust and node tests both read it:
  - `failed`: status failed.
  - `needs_input`: needs-input with `interrupted` true on a running session (a question or an approval mid-turn).
  - `needs_input`: any status but working, when `TabView.question` is set (a finished turn that asked the user, before or after Claude Code's idle notice, and after a restart).
  - Nothing else. Needs-input with `interrupted` false and no question is the idle notice alone. The global orchestrator's own tab never waits.
- `TabView.question` (`#[serde(default)]`, omitted when empty) is the last sentence outside code ending in `?` of the turn's last reply (`global_orchestrator::question_in`, at most 300 characters). The node sets it on Done and on restore, and clears it on Working and Failed. `Waiting.message` is the question when there is one, else the tab's message.
- `sessions` (`#[serde(default)]`, an `Option`) is every session `overview()` builds from the same snapshots, and `waiting` is those whose state is `waiting_on_input` (kind `needs_input`) or `error` (kind `failed`), built from the same `Session` of each tab, so the two lists never disagree. A `Waiting` takes the session's title and message and the tab's `updated` as `since`. A hub that predates `sessions` sends none, and the node then lists this machine's sessions alone. The `projects` tool drops it from its answer.

### `Session`

```json
{
  "id": "tab-2b-7", "machine": "<id>", "machine_name": "MacBook", "account": "bloom",
  "project": { "key": "github.com/o/client-apps", "name": "client-apps", "path": "/Users/f/client-apps" },
  "role": "worker", "captain": "tab-2b-3",
  "title": "Someone mentioned you in the Slack thread", "branch": "felipega/fix-and-ops",
  "pr": { "number": 418, "url": "https://github.com/o/client-apps/pull/418" },
  "state": "waiting_on_input", "message": "Claude needs your permission to use Bash", "last_activity": 1794249424
}
```

- One per tab on each online machine, every account included, newest `last_activity` first. The global orchestrator's own tab is left out, and an offline machine has none.
- `project` is the folder's project (a worktree's is its repository's): the listed project's `key` and `name`, else `<machine id>:<path>` and the folder's name.
- `role` is `captain` (`Role::Orchestrator`), `worker` (`Role::Worker`) or `thread`. `captain`, on a worker only, is the first Capitão up its `spawned_by` tabs, else its project's Capitão on that machine.
- `title` is `TabView.title`, else `TabView.prompt_title`. A thread with no conversation yet has none.
- `branch`, `pr` and `last_activity` come from the transcript tally the node already runs (`stats::Tally`), sent as `TabView.branch`, `TabView.pr` and `TabView.last_message_at`; `last_activity` falls back to `TabView.updated`. They are top-level `TabView` fields because the wire checker names an untagged struct inside `Option` by its keys, so a new `ThreadStats` field reads as a removed variant.
- `message` is set only for `waiting_on_input` and `error`: `TabView.question` when there is one, else the tab's message.

`SessionState::of(&TabView)`, in order. The first two are `waiting_kind`, so a session waits exactly when the sidebar's `waitingKindOf` says it does; `waiting_cases.json` holds the cases, and `a_session_waits_on_input_or_is_in_error_exactly_when_it_waits_on_the_user` checks them against the states:

| State | From the tab |
|---|---|
| `error` | `waiting_kind` is `failed`: status failed, running or not |
| `waiting_on_input` | `waiting_kind` is `needs_input`: a question or an approval mid-turn, or a finished turn whose last reply asked the user (`TabView.question`), a loose thread included |
| `settled` | its session is not running |
| `running` | status working, or `orchestrator_turn` |
| `idle` | everything else that runs: done, the idle notice, no status yet |

## 6. Session tools

The per-session MCP server (`crates/core/src/reminders.rs`) offers tools by `Seat`, which it reads from `PINNED_ROLE`. A delegated child (`PINNED_TASK` set) has no seat.

| Seat | Tools |
|---|---|
| Global orchestrator | reminders (when on) + `memory_search`, `memory_save`, `memory_forget`, `projects`, `send_to_captain`, `read_captain`, `waiting`, `sessions`, `read_session`. No delegation tools, no `agents`. |
| Capitão | everything a plain thread sees + `tell_orchestrator` |
| Anyone else | exactly what they saw before |

`permissions()` lists all eleven project tools, so each is pre-allowed. `Machine::project_tool` enforces the same rules, because the socket takes any tool name:

- `projects`, `send_to_captain`, `read_captain`, `waiting`, `sessions`, `read_session` from another tab: `only the global orchestrator has this tool`.
- `agents` from the global tab: `the global orchestrator does no project work: hand it to the project's Capitão with send_to_captain`.
- `tell_orchestrator` from a non-Capitão: `only a project's Capitão reports to the global orchestrator`.
- `Message::Delegate` from the global tab (`start_delegate`, the `delegate_task` path) is refused with the same text as `agents`, so no child runs in the global folder.

`delegate_task` is the one delegation tool; the old `delegate` tool is gone, with no alias. A child called from a tab is a worker thread: a tab with `Role::Worker` env and `spawned_by` the caller, linked to its task (`Task::thread`). A Claude Code account runs the child in that tab's own session (`Thread::runs`), with the request's model, `--effort` and access; the task ends with the tab's first `Done` (summary = its last reply) or `Failed`, and `task_cancel`, the limit and quit stop it by the session's pid. Cursor and Codex children run headless; their tab stays unstarted with status `working` and shows the task's card, then takes the task's end state and resumes its conversation. Closing the parent leaves a child thread running; closing the child thread cancels its task.

| Tool | Arguments | Result (tool text) |
|---|---|---|
| `projects` | none | `Overview` as one line of JSON |
| `send_to_captain` | `project` (key from `projects`, a folder on this machine, or `<machine>:<folder>`), `message`, `request_id` (new per message; repeats are safe), `machine` (optional id or name, with or without `@`; pass it when `projects` lists the project on more than one machine), `box` (optional `true` or `{ "size" }`: the Capitão runs in an Upstash Box, through `RelayTo::CaptainBox`, with the project's origin as `repo`) | `Handoff` as one line of JSON, or the `captain_box` answer |
| `read_captain` | `project`, `machine` (optional), as `send_to_captain` takes them | the read as one line of JSON, from this machine or through `HubCall::ReadCaptain` |
| `waiting` | none | `Vec<Waiting>` as one line of JSON |
| `sessions` | `project` (optional: a key from `projects`, a name in any case, or a folder), `machine` (optional id or name, with or without `@`) | `{"sessions":[Session],"machines":[OverviewMachine]}` as one line of JSON |
| `read_session` | `id` (a session's id), `machine` (optional), `items` (optional, 20, at most 50) | the read as one line of JSON with `id`, `machine` and `machine_name`: here when this machine has the tab, else through `HubCall::ReadSession` |
| `tell_orchestrator` | `text` | `Handoff` as one line of JSON |

`ProjectTool` variants (`crates/core/src/agent.rs`): `Projects`, `SendToCaptain { project, message, request_id, machine: Option, box: Option<BoxTarget> }`, `ReadCaptain { project, machine: Option }`, `Waiting`, `Sessions { project: Option, machine: Option }`, `ReadSession { id, machine: Option, items: Option }`, `TellOrchestrator { text }`.

Resolution, for `send_to_captain` and `read_captain` alike:

1. `local_project`: when `machine` is absent or names this machine (`is_this_machine`: its hub id or its hostname, with or without `@`), and `project` is a known project folder, `<this machine>:<path>`, or the `repo` of a local project, it is this machine's.
2. Otherwise `remote_checkout` asks the hub for `Overview`. `<machine>:<path>` splits into a folder and a machine, and a `machine` argument wins over the prefix. It keeps the checkouts whose project `key` or `repo`, or whose own `path`, is `project`, on the named machine (its id, or its name in any case). One checkout, or one online checkout among several, wins. Several online checkouts ask for `machine`. A folder the overview does not list goes to the named machine, its name turned into its id through `machines`.
3. Without the hub's overview only `<machine id>:<path>`, or an absolute path with `machine`, names a checkout.
4. A path is absolute when macOS, Linux or Windows would read it so (`/…`, `\\…`, `C:\…`, `C:/…`), whatever this machine runs, so `<machine>:C:\w\site` names a folder on a Windows machine from a Mac. A machine whose name is one letter can't be a `<machine>:` prefix, because it reads as a drive.

### `delegate_task` on another machine or in a box

`target.machine` (an id or name of the account's machines) or `target.box` (`true` or `{ size, repo }`) runs the worker elsewhere. The calling node keeps the task under its own id (`Task::remote`, a `RemoteTarget` with the machine's hub id, its name and the thread there) and sends `HubCall::Delegate { tab, task, request, repo, access, delegator, limits }`, with `repo` its checkout's origin (or `box.repo`), `access` the caller's ceiling, and `limits` the caller's delegation limits. The caller counts the task against Workers at once and admits its effort and time limit before it sends; the target holds the worker to `limits` and records it under `delegator`. Only a tab delegates elsewhere; a headless child is refused.

- Machine: the hub (`crates/server/src/remote.rs`) finds the machine among the account's own, refuses this machine itself and an offline one by name, and calls `Call::Delegate { origin, request, repo, token }` there, `origin.machine` stamped from the asking socket and `token` the account's GitHub token. The target finds its checkout of `repo` (`find_folder`, cloning when it has none) and opens the Worker thread as for a local child, with `Tab.origin` set: its `TabView` shows `spawned_by` = the caller's tab and `spawned_on` = the caller's machine, and its local `spawned_by` stays empty. Its registry task's parent is `remote:<machine>:<tab>` and its `client_request_id` the caller's task id, so a repeat starts nothing.
- Box: `Boxes::open_delegate` with the Workers permission (`BoxAccess.worker`; Ask files `workers of <machine> <tab>`), the account's connection and cap, a row with `connector = "delegate"` and `BoxWork::Delegate`, and the Claude token the account keeps for `target.providerInstanceId` (`Connectors::box_claude_token`; none takes the connection's own). It answers `{"machine","machineName","box","workState":"working"}` at once and sends `Call::Delegate` once the box's node is online. A boot that fails ends the task on the caller with `Call::RemoteEnded`.
- Ends: each delegate pass on the target sends every finished task of an origin as `HubCall::RemoteEnded { to, tab, task, end }`; the hub passes `Call::RemoteEnded { from, ... }` on. The caller settles only its own working task, owned by that tab and placed on `from`, and types the usual notice. An end the hub could not pass on is sent again every 3 s. A worker box goes once its end got through.
- Status and cancel: `task_status` on the caller asks the target through `HubCall::RemoteTask` / `Call::RemoteTask` at most every 3 s, adds its `recentSteps`, and settles a finished answer. `task_cancel` forwards the cancel; a worker box is deleted with it, booting or not.
- Box budget: a worker box and a Captain box take the Box budget of the project's taste (`boxes::box_budget`), as a session box does.
- Record: the machine that ran the worker records it, with `target` `machine` (the box rule below turns it into `box`); the caller's copy, which holds `Task::remote`, leaves none.

### Delegation metrics

Every delegated task leaves a `TaskRecord` (`crates/core/src/delegate/metrics.rs`) when it ends: delegator (`kind` global_orchestrator, captain, worker or session, with tab and project), provider, account, model, cost tier, effort, target (local, machine for a worker another machine's thread delegated, or box), start and end, `usage` (the limits unit's `usage::TaskUsage`: tokens in and out, `cost_usd` when known, `wall_ms`, `box_usd` and `box_ms`), `outcome` (done, failed, limit, cancelled) with `reason` and `limit` (`time` or `budget`), and `review`. The node keeps it on the task in its delegates file and sends unsent ones with `RecordTasks` from the delegate pass and on each hub connect. The hub keeps them in `delegation_tasks`.

- Usage: a Claude thread child is metered from its transcript by `usage::Meter` (the limits unit), read once more just before the task settles. A headless child counts the `usage` of its stream's `result` / `turn.completed` lines; only Claude prints a cost there, so Cursor and Codex records have `cost_usd: None`.
- Box work: a record from a machine that runs in one of the account's boxes is billed the box time since that box's previous task ended (the first task: since the box was made), at `BOX_USD_PER_HOUR`. A record sent again keeps that figure.
- Accepted or redone: `task_review(taskId, verdict, note?)` sets it. pinned sets it by itself in two cases and never over the delegator's verdict: the pull request a task's thread watched merged (accepted), and the same caller starting a task with the same title again (the newest earlier one becomes redone).
- `delegation_stats(days?)` (every seat but a delegated child, the global orchestrator included) answers `DelegationStats::brief`: per model runs, `doneRate`, `medianMinutes`, `medianUsd`, `stoppedByLimit`, accepted and redone, plus `scope` (`every machine of the account`, or `this machine` without a hub or with one too old). `task_review` goes to whoever can delegate.

## 7. The delivery primitive and the ledger

`Machine::enqueue_to_captain(path, text, request_id, sender, busy: Option<BusyPolicy>) -> Result<Handoff, String>` and `Machine::deliver_report(request_id, &CaptainReport) -> Result<Handoff, String>` share `hand_to(tab, line, request_id, busy, &ledger)`.

`hand_to` decides from the tab under one lock:

| Tab state | Action | Outcome |
|---|---|---|
| no session, not starting | set `launch` to resume args (when it has a conversation) + the line as the positional prompt, `start_claude` | `iniciado` |
| session running, no conversation, no status, nothing typed or held, not started by the ledger | same launch args, `relaunch` | `iniciado` |
| raw session, `notice_idle()` | `write_notice` (typed under the tab lock) | `entregue` |
| anything else, or `write_notice` lost the race | `BusyPolicy::Refuse`: nothing. `Queue`: `Delegates::queue_request(tab, claude session, line, request_id)` + `arm_delegate_pass` | `ocupado` / `na_fila` |

- Starting on a positional prompt keeps a startup dialog, such as folder trust, from taking a typed Enter.
- A Capitão the ledger started is busy until its first hook, so a second message queues instead of relaunching it.
- The queue is the existing worker-report queue (`registry.rs`). It drains when the tab is idle (`maybe_deliver`, the 1 s delegate pass) or as hook context on a Stop or a prompt submit (`take_notices`). Several queued items go out as one paste joined by `\n`. It survives a restart and follows the Capitão's Claude session to its new tab id (`rebind`).

`Ledger` (`<config dir>/captain-requests.json`, newest 500 kept) holds one `Sent { request_id, sender, project, handoff, progress, at }` per handed message.

| `Progress` | When |
|---|---|
| `queued` | `na_fila` |
| `in_flight` | `entregue` or `iniciado` at once. A queued one when the drain types it: `drop_claimed_reports` returns the request ids typed, and `maybe_deliver` and the hook listener mark them. |
| `answered` | `Ledger::answer(project)` at the end of the turn. The reports unit calls it. |

- A repeat `request_id` answers `sent.handoff` and types nothing. A repeat for another project, or for the global folder when it was a Capitão message, is an error.
- Reports to the global tab use the same ledger with `project` = the global folder. Use ids that cannot collide with message ids, such as `report:<uuid>`.
- Lock order: `sent` before `tabs`. Never lock `sent` while holding `tabs`.
- Ordering at a turn end (`Machine::listen`, `record_event`): `take_notices` claims the queue, `record_event` runs `captain_turn_ended` for the turn that ended, and only then are the claimed request ids marked `in_flight`. A Stop hook that hands a queued message over continues the turn, and that message belongs to the next turn end, not this one.

## 8. Text formats

**Message to a Capitão.** `[<sender>] <text>`, for example `[Orchestrator] Ship the form.` The sender is the cleaned settings name, so it has no brackets.

**Report to the global orchestrator** (`CaptainReport::line`):

```
[pinned] Capitão <project> (<machine name>) [<machine id> <tab id>]: <text>
```

- The project and machine names have `(`, `)`, `[`, `]`, `:` and control characters replaced by spaces. An unknown machine id is `-`. The text may span lines.
- UI regex for the header: `/^\[pinned\] Capitão ([^()\[\]\n]+) \(([^()\[\]\n]*)\) \[(\S+) (\S+)\]: /m`. The machine id and tab id link to the thread (`machine` + `tab`). The UI also reads `Captain` in place of `Capitão`; the agents keep writing `Capitão`.
- One typed prompt may hold several reports, joined by the queue with `\n`. Each report starts at a line that matches the header and runs to the next header line or the end.

**Quick replies.** The orchestrator's prompt (node-local unit) ends a reply that asks Felipe something with a last line:

```
Opções: Sim | Ainda não
```

- UI: when the last non-empty line of an assistant message matches `/^Opções: (.+)$/`, split the capture on ` | `, hide the line, and render one button per option. A click types that option's text as the user's message. `Options:` in place of `Opções:` reads the same; the agents keep writing `Opções:`.

## 9. Seams and the unit that filled each

All in `crates/core/src/machine.rs` unless noted. Every seam is built.

| Seam | Signature | Built by | What it does |
|---|---|---|---|
| `projects_overview` | `fn projects_overview(&self) -> Result<global::Overview, String>` | node-local, remote-reach | The hub's `Overview` when linked. This machine alone (`local_overview`, under its hub id) while unlinked or when the hub is too old. |
| `waiting` | `fn waiting(&self) -> Result<Vec<global::Waiting>, String>` | node-local, sessions | `projects_overview().waiting`: the sessions waiting on input or in error (section 5). |
| `read_captain` | `fn read_captain(&self, project: &str, machine: Option<&str>) -> Result<String, String>` | node-local, remote-reach | `reading::read` of the Capitão here, or `HubCall::ReadCaptain` to the machine `remote_checkout` finds. |
| `route_remote` | `fn route_remote(&self, project: &str, machine: Option<&str>, text: &str, request_id: &str, sender: &str) -> Result<Handoff, String>` | hub-bridge, remote-reach | `HubCall::Relay { to: captain }` to the checkout `remote_checkout` finds, under this machine's busy policy. |
| `remote_checkout` | `fn remote_checkout(&self, project: &str, machine: Option<&str>) -> Result<(MachineId, String), String>` | hub-bridge, remote-reach | Section 6 resolution, steps 2 and 3. |
| `report_up` | `fn report_up(&self, captain: &str, project: &Path, text: String, answers: Vec<String>) -> Result<Handoff, String>` | node-local, remote-reach | Builds `CaptainReport` with this machine's hub id, then `deliver_report` when `settings.host`, else `HubCall::Relay { to: orchestrator }`. |
| `captain_turn_ended` | `fn captain_turn_ended(&self, captain: &str, project: &Path, event: &AgentEvent)` | node-local | `Ledger::answer(project)`, then under `ReportPolicy::Auto` the turn's last reply goes up through `report_up` on its own thread. `thread::last_reply` reads it from the transcript's end and stops at the turn's prompt, so a turn that wrote no text reports `(no reply)`, never an older reply. |
| `is_this_machine` | `fn is_this_machine(&self, machine: &str) -> bool` | hub-bridge | Matches the id from `ToMachine::Hello` (`set_hub_id`) or the hostname, with or without `@`. |
| hub handle | `Machine::set_hub(...)`, `Machine::call_hub(HubCall)` | foundation | Installed by `link::spawn`. `Err("this machine is not linked to a hub")` when unlinked. |
| hub calls | `crates/server/src/relay.rs` `serve` | hub-bridge (`Relay`), remote-reach (`Overview`, `ReadCaptain`) | Section 4. |
| global prompt | `launch_tab` | node-local | The global orchestrator's prompt, and the Capitão's line about `[<name>]` messages and `tell_orchestrator`. |

## 10. Files each later unit mainly touches

| Unit | Files |
|---|---|
| node-local | `crates/core/src/machine.rs` (global prompt in `launch_tab`, `projects_overview`, `waiting`, `read_captain` local, `report_up`, `captain_turn_ended`), `crates/core/src/agent.rs` (prompts), `crates/core/src/global_orchestrator.rs` (prompt text, overview builders) |
| hub-bridge | `crates/server/src/http.rs`, `crates/server/src/hub.rs`, `crates/server/src/mcp.rs`, `crates/core/src/link.rs`, the remote seams in `machine.rs` (`route_remote`, `is_this_machine`, the hub handle), `crates/server/tests/hub.rs` |
| UI-chat | `ui/gui-sidebar.js` (fixed Orquestrador row from `orchestrator_global`, "Esperando você (N)"), the timeline files (report grouping by the header in section 8, quick-reply buttons) |
| UI-settings | the settings page (`ui/gui-orchestration.js` or a new file: `Call::GlobalOrchestratorSettings` / `SetGlobalOrchestratorSettings` on each machine, one `host`), the projects page, the memory panel (`ui/gui-surfaces.js`: `Call::Memory*` with `path` = the global tab's `cwd`) |
| remote-reach | `crates/server/src/relay.rs` (`Overview`, `ReadCaptain`), `crates/server/src/hub.rs` (`Account::listed`, `tabs_of`), `crates/core/src/machine.rs` (`remote_checkout`, `read_captain`, `local_overview`, `report_up`, `mark_typed`), `crates/core/src/global_orchestrator.rs` (`Ledger::turns`, `Overview.machines`), `crates/core/src/wire.rs` (`TabView.orchestrator_turn`, `HubCall::ReadCaptain`) |

`machine.rs` is shared by node-local and hub-bridge. Each fills only its own seams to keep merges clean.

## 11. Rules every unit keeps

- Wire: add variants or `#[serde(default)]` fields only. After a wire change run `snapshot current`, then `check --base origin/main` must list no New breaks. Plain `check` exits 1 on main already, on old 0.8 to 0.12 breaks filed under Already shipped.
- Never type into a busy Capitão. Use `enqueue_to_captain` or `hand_to`; never call `type_prompt` on a Capitão directly.
- Keep the lock order `sent` then `tabs`.
- Keep request ids idempotent end to end. The hub passes the orchestrator's `request_id` through `Relay` to `Enqueue` unchanged.

## 12. Known gaps

- A Capitão running with no conversation that the user is looking at is relaunched with the message as its first prompt. Nothing is lost, because it has no conversation, but its screen restarts.
- When a Capitão tab is closed while a message waits in its queue, the message stays queued for a tab that is gone until a later restart rebinds it by session. The ledger keeps it `queued`.
- Without a running global tab on the host, a report starts it with the report as the first prompt.
- A Capitão that `hand_to` starts with the message as its first prompt (`iniciado`) keeps that message only in its launch arguments until its first hook gives it a Claude session. When pinned quits in that window, for example while a folder-trust dialog waits, the tab is not saved, the message is lost, and the ledger still answers `iniciado` for its `request_id`. A message queued in the same window (`queue_request` with no session) stays bound to the dead tab id, like the closed-tab case above.
- A stopped Capitão restored with `Continue from where you left off.` as its launch prompt loses that prompt when `hand_to` starts it on a message. Nothing is lost from the conversation. The node starts restored tabs at once, so the window is short.
- `local_project` prefers this machine when `machine` is absent. The global prompt tells the orchestrator to pass `machine` when `projects` lists more than one checkout, per the routing rule (running Capitão, then own machine, then first online).
- A Capitão whose session ends mid-turn with no Done or Failed hook keeps its message `in_flight`. Once it runs again it reads `trabalhando`, and a message it started still makes the next one queue, until its next turn ends.
- The hub keeps each machine's project list in memory only. After a hub restart, an offline machine is in `machines` with no projects until it comes online and an overview lists them.
- Every `projects` and `waiting`, and every `send_to_captain` or `read_captain` for another machine, runs one overview: `ListProjects` and `GlobalOrchestratorSettings` to each online machine, plus `Call::Tabs` for one that has not reported tabs. A `send_to_captain` or `read_captain` then asks each online machine's settings once more for the host check. Calls queue behind a machine's other calls, so a busy machine may show its last project list with `não respondeu`.
- The hub refuses a non-host machine's `Overview` only after the overview's calls, since it reads the host from their settings answers. The refused machine gets no projects, but every machine answered the calls.
- A machine computes its tab views and sends them in two steps (`emit_tabs`), so two threads that emit at once may deliver the older views last. The hub then holds a stale `orchestrator_turn` or `status` until the machine's next emit, for example the Capitão's next hook. `hand_to` decides from the machine's own tabs, so routing is unaffected. This predates the global orchestrator.
- A report delivered on the host carries the host's hostname as `machine_name`, and a relayed one the hub's name for the machine. They differ when the machine was renamed on the hub.

## 13. Agent capability surface (0.17.22)

Defined by unit P1, branch `parity/surface`. Every session pinned launches reaches what the user can do in the app through two MCP tools, scoped to the hub account that owns its machine, written to an audit trail, and with the user's approval in the app for destructive or credential actions. P2 (`parity/session-marks`) and P3 (`parity/surface-rest`) register and wire their actions here.

### 13.1 The manifest

`crates/core/src/surface.rs` holds `ACTIONS: &[Action]`, one entry per `Call` and `HubCall` variant.

```rust
pub enum Side { Node, Hub }                         // Call or HubCall
pub enum Class { Read, Write, Confirm, Excluded }
pub struct Action {
    pub name: &'static str,     // what act takes
    pub side: Side,
    pub cmd: &'static str,      // the variant's wire tag
    pub class: Class,
    pub wired: bool,            // act runs it; false refuses it as not wired yet
    pub starts_work: bool,      // held to the delegation limits (13.6)
    pub about: &'static str,    // the description agents read; for an excluded one, why
    pub hides: &'static [&'static str], // reply fields agents never get
    pub check: Option<fn(&Parsed) -> Result<(), String>>, // refuses arguments that parse
}
```

- An action is named by its wire `cmd`. Where a `Call` and a `HubCall` share a `cmd`, the `HubCall` keeps the name and the `Call` is `node_<cmd>`. Today that is `clone` / `node_clone` and `read_session` / `node_read_session`.
- Arguments are the variant's fields, as the wire enum's schemars schema describes them, without `cmd`. `act` builds `{"cmd": <cmd>, ...args}` and parses it into the `Call` or `HubCall`. A new action needs no hand-written argument schema.
- `surface::tests::every_wire_call_has_one_action` reads the `cmd` of every variant from the schemars schemas of `Call` and `HubCall` and fails when one has no entry, or when an entry names no variant. A new variant does not pass the tests until its unit decides its class.
- Other tests: names are unique, an excluded action is never wired, every `read` action is wired, and a `HubCall` with `starts_work` is wired only when `surface::HUB_LIMITS` names it, because the server's `act` runs each of those through its own limits path (`open_box`, `run_workflow`).

**Adding or wiring an action.** Add the variant, then its entry. To wire one: make it work through `act`, prove it with a test that calls `act` as an agent does, then set `wired: true`. An action that starts work also needs its limits path (13.6) before it is wired.

**Argument checks.** When the wire accepts arguments an agent must not send, give the entry a check with `.checks(f)`, where `f: fn(&Parsed) -> Result<(), String>` lives in the unit's own module. `parse` runs it on the call it built, after serde, so `f` matches its own variant and answers `Ok(())` for anything else. Its error is the refusal text the agent gets and the audit row keeps, so start it with the action's name. Example: P2's `mark_sessions` refuses changes to an archive entry with `h("mark_sessions", Write, "…").wired().checks(marks::agent_check)`. Test it through `act`, as for wiring.

### 13.2 Classes

| Class | What `act` does |
|---|---|
| `read` | Runs it and answers the result. |
| `write` | Runs it and answers the result. |
| `confirm` | Runs nothing. It becomes a confirmation the user approves or denies in the app (13.5). |
| `excluded` | Refuses it. UI plumbing, a session's own shell and files, git and gh, machine-to-hub plumbing, and the surface's own calls. |

Every refusal, run and confirmation writes one audit row (13.4).

### 13.3 The two tools

Offered to every seat (`Audience::Everyone` in `reminders.rs`), so the global orchestrator, a Capitão, a worker, a thread, a taste chat and a workflow session get them. Both need `PINNED_TAB`: a headless delegated child (Cursor, Codex) has no tab and its parent acts for it. Both are in `permissions()`, so Claude Code does not ask. The socket waits 70 s for them.

| Tool | Arguments | Result (tool text) |
|---|---|---|
| `actions` | `search` (optional: words matched against name and description) | `{"actions":[{"name":"rename_machine","class":"write","on":"hub","description":"…","args":{JSON Schema}}]}`, every wired action that is not excluded. `on` is `machine` for a `Call` (act takes `machine`) or `hub`. Read on the node from the manifest; no hub call. |
| `act` | `action`, `args` (object, default `{}`), `request_id` (optional), `machine` (optional id or name, for an `on: machine` action) | `ActAnswer` as one line of JSON. A refusal is a tool error with its reason. |

`ProjectTool::Actions { search: Option<String> }` and `ProjectTool::Act { action, args: Value, request_id: Option<String>, machine: Option<String> }` carry them on the socket. The node sends `HubCall::Act`; without a hub it answers `this machine is not linked to a hub, which keeps the audit trail act writes`.

`ActAnswer`, tagged by `outcome`:

```json
{"outcome":"done","result":<the call's answer, redacted>}
{"outcome":"pending","id":"<confirmation id>","next":"The user approves or denies this in the app. A notice reaches this session when they decide."}
{"outcome":"repeat","earlier":"done","at":1794249424}
```

`repeat` answers a `read` or `write` that the same session (machine and tab) already sent with the same action and `request_id`, without running it again. Another session's or another action's `request_id` is its own. A `confirm` with a used `request_id` answers its first `pending` (or the decision, once made).

### 13.4 How the hub runs an act

`crates/server/src/surface.rs` `act(hub, account, origin_machine, AgentAct)`. `HubCall::Act` is taken only from a machine; a client gets `only an agent session on a machine acts`.

1. The caller is the tab `act.tab` in the tabs the origin machine last reported. None: refused, `<machine> runs no session <tab>`. The hub reads the role and the Claude session from that `TabView`; the node sends neither.
2. Role: `global_orchestrator` (`orchestrator_global`), `captain` (`Role::Orchestrator`), `worker` (`Role::Worker`), `taste_chat`, `workflow` (a `connector` tab), else `session`.
3. Refusals, in order: no such action (`no action <name>; call actions to list them`), excluded (`<name> is not open to agents: <about>`), not wired (`<name> is a <class> action that is not wired yet`), a secret in the arguments (13.7), arguments that don't parse (`<name>: <serde error>`) or that the action's check refuses (13.1), a `starts_work` action on another machine (`<name> starts work only on this session's own machine for now`).
4. `confirm`: a row in `agent_confirmations`, unique per (account, machine, tab, action, `request_id`), and `ToClient::Confirmations` to the account's clients. A `machine` named for an action on a machine is resolved now and kept as its id, so a later rename can't move the action. Answers `pending`.
5. `read` / `write`:
   - `HubCall`: `Hub::hub_call(account, call)`, the path a signed-in client's call takes.
   - `Call` on the origin machine (no `machine`, or `machine` naming the origin): `Call::ActFor { caller: act.tab, call }` to the origin machine.
   - `Call` on another machine of the account (`machine` is its id or its name, any case): the plain `Call` to that machine, as the app sends it, except `prompt`, `open_global_orchestrator`, `restart_claude`, `set_model`, `set_bypass`, `rewind`, `switch_account` and `set_workspace`, which go as `Call::ActFor { caller: "", call }` so the checks `ActFor` makes of the target still apply and the ones it makes of the asking session refuse. A machine of another account reads as `this account has no machine <m>`.
   - A saved workflow with a session that skips its permission prompts (`permissionMode` or a step's `permission` of `bypassPermissions`) is refused for a caller whose own session asks for them (`surface::skips_prompts`), and its audit row notes the widening.
6. The answer goes through `surface::redact` and the action's `hides`, then back as `done`. An error, and a refusal, go through `surface::mask_secrets`, then back as the tool error.

The audit row of something that already ran is written after it runs. When that write fails, the hub logs it and still answers, so the session does not run the action twice.

**Audit trail.** Table `agent_actions (id uuid, account, machine, tab, role, action, args text, outcome text, detail text, request_id text, at timestamptz)`, indexed on (account, at). `args` is `surface::summary`: the arguments as compact JSON, secrets replaced by `[hidden]`, strings cut at 200 characters, the whole at 1000. `outcome` is `done`, `failed`, `refused`, `pending`, `approved` or `denied`. `detail` is the error or refusal. For an act that loosened a session's permissions it says so (`surface::widening`): `loosened permissions: <tab> now skips its permission prompts` for `set_bypass` with `on: true`, and `loosened permissions: the new tab in <path> skips its permission prompts` for `open_tab` with `bypass`.

`AuditEntry` (outbound root):

```json
{"id":"<uuid>","at":1794249424,"machine":"<id>","machine_name":"MacBook","tab":"tab-2b-7","role":"worker","action":"rename_machine","args":"{\"id\":\"…\",\"name\":\"Mac mini\"}","outcome":"done","detail":"","request_id":"r-1"}
```

`machine_name` is empty for a machine since removed. `request_id` is omitted when absent.

### 13.5 Confirmations

Table `agent_confirmations (id uuid, account, machine, tab, session text, role, title text, action, args jsonb, request_id text, created_at, decided_at, approved bool, outcome text, notified bool, target text, result jsonb)`, unique (account, machine, tab, action, request_id). It lives on the hub, so a node restart loses nothing.

`Confirmation` (outbound root):

```json
{"id":"<uuid>","at":1794249424,"machine":"<id>","machine_name":"MacBook","tab":"tab-2b-7","title":"Clean up old machines","role":"worker","action":"remove_machine","about":"Removes a machine from the account…","args":{"id":"<machine id>"},"request_id":"r-9","decision":"approved","decided_at":1794249500,"outcome":"done"}
```

`decision`, `decided_at` and `outcome` are absent while it waits. `target` (the machine's id) is present when `act` named another `machine` for an action on a machine; approving runs it there. The table also keeps the redacted result for the notice.

| Wire | From | Answer |
|---|---|---|
| `HubCall::Confirmations` | a client or a machine | `Vec<Confirmation>`: every waiting one, oldest first, then the last 20 decided, newest first. |
| `HubCall::DecideConfirmation { id, approve }` | a signed-in client only, on a connection the owner unlocked with the password (13.11). A machine gets `only the account's owner decides, in the app`. The manifest excludes it. | `Decided`: `{"decision":"approved","outcome":"done","result":<unredacted answer>}` or `{"decision":"denied","outcome":"denied"}`. Error `no confirmation <id> waits` for an unknown, decided or other account's id. |
| `ToClient::Confirmations { items }` | hub to client | The waiting ones, after `Hello` and after every change. An older app ignores the frame. |
| `HubCall::AgentAudit { before?, limit? }` | a client, or an agent through `act` | `Vec<AuditEntry>` as 13.8 says. |

Deciding:

1. The row is taken under one statement (`decided_at is null` and the account matches), so two clicks decide once.
2. One audit row for the decision, `approved` or `denied`, written before anything runs.
3. Approve runs the stored action as 13.4 step 5 does, for the original tab and machine, records `outcome` (`done`, or `failed: <error>`), and writes a second row for the run, `done` or `failed` with the error. Deny runs nothing.
4. `ToClient::Confirmations` again.
5. A notice to the calling session: `Call::Notice { tab, session, text, id }` to the origin machine. The text is `[pinned] The user approved your <action> request (<id>): <outcome>` plus the redacted result, or `[pinned] The user denied your <action> request (<id>). It did not run.` `notified` turns true only when the node answers `true`. When the machine is offline, the call fails, or no tab runs the session yet, the hub tries again 3, 10, 30 and 90 s after that machine next connects, while its tabs come back. A notice older than a week is not sent again.

`Call::Notice` queues the text for the tab with `Delegates::queue_report`, the queue worker reports use: typed once the session is idle, or added to the hook context at its turn's end. When `tab` is gone, as after a node restart, the node finds the tab running Claude session `session`. A repeat `id` queues nothing. Answers `true` when it queued, `false` when no tab has that session.

`HubCall::CreateInvite`'s `hides` is `["code"]`: the agent learns the invite's id and expiry, and the user sees the code in the app's answer to Approve.

The global orchestrator's `waiting` tool lists the waiting confirmations too, as `Waiting { kind: "confirm", machine, machine_name, tab, title: "Approve <action>?", message: <args summary>, since }` with no `project`. They come from `HubCall::Confirmations` on the node, never from `Overview`, so an older node never reads `WaitingKind::Confirm`.

### 13.6 Limits on work an action starts

`Call::ActFor { caller: TabId, call: Value }` runs `call` for the agent session in `caller` on this machine. `call` is a `Call` as JSON (a `Value`, so the enum does not contain itself).

- `start_claude` (the action P1 wires through limits): the target tab becomes the caller's worker. `Delegates::claim` under the caller's project's workers-at-once limit (`caller_limits`), with the time limit and budget held to it as `delegate_task`'s are, the tab linked as the task's thread (`runs: true`, `task`, `spawned_by: caller`). Past the limit it answers the same refusal as `delegate_task`. Its first finished turn ends the task, the caller gets the task's notice, and the task's `TaskRecord` goes to the hub's metrics. A tab already running, or the thread of a task still working, answers its folder and claims nothing. A tab whose task ended claims a new one. It is refused, as `delegate_task` would be:
  - for the global orchestrator: `the global orchestrator does no project work: hand it to the project's Capitão with send_to_captain`;
  - for a tab outside the caller's project (`history::project_of`), so a subfolder can't count apart: `<tab> works in <folder>, outside your project <project>: start_claude starts workers only in your own project`;
  - for a tab with wider access than the caller's (`child_access`, `tighten`): `<tab> runs with full access, wider than your session's auto`.
- `enqueue` (a message that starts a stopped Captain): `enqueue_for` signs it as the caller (the `sender` argument is ignored: `[Worker <title>]`, `[Captain]`, `[Session <title>]`, or the orchestrator's name for the global tab) and refuses a Captain's own tab. A Captain that already runs takes the message with no new session. A Captain that does not run starts only under the limits of its project (`limits_in`, narrowed by the caller's own task): `admit` for the Captain tab's model (tier and effort), and `Delegates::working_in(project)` below workers-at-once (`workers_full`). A Captain is not a worker, so it has no `TaskRecord`.
- `prompt` types into a running session of the account (`may_prompt`). It refuses the caller's own tab, a Captain (`enqueue` hands it a message when it is idle), the global orchestrator (`tell_orchestrator`) and a taste chat. It starts nothing: a tab with no session answers `no running session`.
- `open_global_orchestrator` runs only for the global orchestrator's own session.
- Any other action with `starts_work: true` is refused by `ActFor` until its unit gives it a limits path: `<name> starts work, and nothing holds it to the limits yet`.
- `restart_claude`, `set_model`, `set_bypass`, `rewind`, `switch_account` and `set_workspace` also start a stopped tab, so `ActFor` runs them only on a tab whose session runs: `<cmd> restarts a running session, and <tab> runs none: start_claude starts it as your worker`.
- `set_bypass` stays `write`: permission changes run directly, and loosening goes to the audit trail (13.4). With `on: true` it runs only for a caller with full access, so an agent never grants more than it has: `set_bypass on skips a session's permission prompts, and your own session asks for them`.
- Every other call runs as `Machine::call` runs it. `ActFor` waits as long as its inner call (`may_wait`).

### 13.7 Secrets

- An argument object holding a non-empty string under a key named `key`, `token`, `password`, `secret`, `api_key`, `claude_token`, or ending in `_key`, `_token`, `_secret` or `_password`, at any depth, is refused: `an agent never sends a password, key or token: <field>. The user enters it in the app.`
- An action may list dotted paths it takes under such a name (`Action::sends`, `.sends(&["project.key"])` for `share_project`, whose project `key` names a repository). The other fields of its arguments are still checked.
- `surface::redact` replaces, at any depth of an answer and of an audit summary (and `surface::mask_secrets` in every error and refusal): the values of keys named `token`, `password`, `secret`, `api_key`, `claude_token` or ending in `_token`, `_secret`, `_password`; and any string that holds something shaped like a secret (`sk-`, `ghp_`, `gho_`, `github_pat_`, `lin_api_`, `xoxb-`, `xoxp-`, `AKIA` and 16 more, a JWT, a PEM private key) with that part as `[hidden]`.

### 13.8 What P1 wires

Every `read` action, plus `rename_machine`, `memory_save` and `memory_forget` (any project of the account, through `path` and `machine`), `open_tab`, and `start_claude` (13.6). Confirm: `remove_machine` and `create_invite`. P2's `session_marks` is a read, so it is wired too. P2 wired the session actions (13.10). New reads: `agent_audit { before?, limit? }` (`Vec<AuditEntry>`, newest first, 50 by default, at most 200, the account's own rows only, from unix second `before` and earlier, so a page repeats the rows of its boundary second and the client drops them by id) and `confirmations`.

P2 and P3 wire the rest by flipping `wired` with a test each. Each unit lists the actions it wired in its report.

Classes P1 changed after review, so P2 and P3 wire them as these: `close_tab` is excluded (agents archive or delete a thread through P2's actions). `update_self` (it stops the machine's sessions), `extend_box` (it raises a spending cap), `pair_code` (it adds a machine; `hides: ["code", "command"]`), `set_taste` and `reset_taste_field` (they carry the limits workers run under) are confirm.

### 13.8a What `parity/surface-rest-a` wires

| Area | Actions (class) |
|---|---|
| Workflows | `save_connector` (write; a new workflow with no `id`, an edit with one), `generate_workflow` (write), `delete_connector` (confirm). `get_settings` (read, P1) lists the account's workflows. |
| Projects | `create_project`, `make_folder`, `publish_project`, `set_project_account`, `save_project_scripts` (write, on a machine); `clone` (write, the hub's: it adds the account's own GitHub token, the node's `node_clone` stays excluded), `share_project` (write). The reads `list_projects`, `project_history`, `shared_projects`, `project_scripts` were already wired. |
| Sessions | `open_scratch`, `open_orchestrator`, `open_global_orchestrator` (the global orchestrator's own session only), `prompt`, `enqueue` (`starts_work`, 13.6). `search_threads` (read) was already wired. |
| Settings | `set_global_orchestrator_settings`, `set_usage_prices`, `save_skill` (write). `set_taste` and `reset_taste_field` stay confirm, since they carry the delegation limits. |
| Memory, usage | Already wired by P1: `memory_status`, `memory_search`, `memory_save`, `memory_forget`, `token_usage`, `usage_limits`, `delegation_stats`. |

The `Run` and `Deliver` calls are how the hub opens a workflow's session and types into it, and stay excluded. 0.17.23 adds a manual run, for the app and for agents (13.12).

### 13.9 Account isolation

Every new query filters on the account of the socket that asked: `agent_actions`, `agent_confirmations`, `Confirmations`, `DecideConfirmation`, `AgentAudit`. `crates/server/tests/hub.rs` proves that account B's agent can't read A's audit rows or confirmations, act on A's machines, or decide A's confirmations, and that B's client can't either.

### 13.10 What P2 wires: the session actions

Unit P2, branches `parity/session-marks` (storage) and `parity/session-actions` (this wiring). Each runs through `act`, is `write` unless noted, and has a test through `act` (core tests on a started tab; hub tests with two accounts).

| Action | Does | Notes |
|---|---|---|
| `stop_turn` | Stops the turn, as the stop button does. | |
| `restart_claude`, `set_model`, `set_bypass`, `rewind`, `switch_account`, `set_workspace` | Restart a session on its conversation, with the change. | Run only on a tab whose session runs (13.6): they would otherwise start a stopped tab outside the limits. `set_model` is also held to the caller's project tier. `set_bypass` stays `write`, `on: true` needs a caller with full access, and the audit row says `loosened permissions`. |
| `fork`, `set_folder` | Open a tab, not started, on a copy of a conversation; move a tab that has not started. | Start nothing. |
| `regenerate_title` | Names a conversation again (a headless Haiku call). | Starts no session. |
| `mark_sessions` | Rename, pin, order, settle, snooze, unread and auto-settle marks, in the hub, for every device. | `marks::agent_check` (`.checks`) refuses `clear` and `archived`, which only the app's own Archive and Delete write. |
| `archive_session` | Archives the conversation in a tab; its tab closes and Settings › Archive keeps it. | Refused while its turn runs. |
| `unarchive_session` | Reopens an archived conversation in a new tab with its name and pin back, not started (`Call::OpenRun`). | The user's own Unarchive resumes it at once. An agent resumes it with `start_claude`, under its limits. |
| `delete_session` | Closes the tab and drops the conversation's marks. | `confirm`: it waits in Waiting on you, runs once on approval, and writes `approved` then `done` or `failed`. `close_tab` stays excluded. |

`Call::OpenRun { run: RunSpec }` is the hub's way to open a tab like `Run` but not start it. It is excluded: only the hub sends it.

### 13.11 Only the owner approves

The web app keeps its login in `localStorage`, and a desktop app older than 0.17.23 keeps it in `account.json`. Any process of the same OS user can read both, so a login alone does not prove that the owner is there. The password proves it, because nothing stores it.

- `ClientFrame::Auth { token, lock?, unlock? }`. An app that sends `lock: true` locks its login for good (`logins.locked`). An older app leaves it out and keeps every call it had.
- `surface::owner_only(side, cmd, locked)`. `decide_confirmation` always needs an unlocked connection, and so does every `confirm` action on a locked login. Without one the hub answers `surface::LOCKED`: `unlock with your password in the app to do this`.
- `HubCall::Unlock { password }` comes from a signed-in client only. The manifest excludes it, and a machine is refused. The hub checks the password, at most 10 tries per login and 100 per account in 15 minutes, and answers `Unlocked { token, until }`. The connection counts as the owner until `until`, 12 hours later. The app keeps `token` in memory and sends it in `Auth.unlock` after a reconnect. The token works only with the login it was taken on. A hub restart forgets it, and the app asks for the password again.
- `ui/hub.js` asks for the password when a call answers `LOCKED`, once for every call waiting on it, and then sends the call again.
- `POST /api/machines` refuses a locked login. An app adds its machine before it locks the login.
- A node keeps only its machine token. `pinned-server login` ends the login after it adds the machine. A node that finds a login an older pinned kept in `account.json` removes it from the file and ends it at the hub.
- A hub that stopped between a decision and its outcome leaves the confirmation decided with no outcome. When a hub starts, and again 5 minutes later, it gives every such confirmation decided more than 5 minutes earlier an outcome: `denied`, or `failed: the hub restarted before it could tell whether this ran; check before asking again` with a `failed` audit row. A younger one may still be running on the hub that a deploy replaces, so the hub leaves it alone. The session gets its notice at once, or when its machine connects.

A login read off a disk still runs every other client call: it reads sessions, types into tabs and starts sessions on the account's machines. After this change, the machine that runs the desktop app is the only one that keeps a login.

### 13.13 The app's login in the OS credential store (0.17.23, #126)

- `login_store::LoginStore` is the store's interface: `get`, `set` and `delete` by account. `login_store::system()` answers the macOS Keychain or the Windows Credential Manager, and `None` on other systems, where `account.json` keeps the login as before.
- `login_store::system()` answers the macOS Keychain only in a build signed with a Developer ID: `scripts/apple-signing-env.sh` prints `PINNED_DEVELOPER_ID_BUILD=1` when it exports the signing identity and certificate, and `login_store::keychain_on` reads that compile-time flag. An ad-hoc build answers `None`, so the login stays in `account.json` as in 0.17.22: the Keychain item would name that build's cdhash and every update would ask for the Mac password. The Windows Credential Manager never prompts and ignores the flag.
- In a Developer ID build, the login on macOS is a generic password in the login keychain: service `dev.personal-harness.pinned.hub-login`, account `<hub URL> <account email>` (`Credentials::login_account`). The app creates it with the default access list, so another program reads it only through the user's password prompt. `docs/release/developer-id.md` says what changes once the app is signed with a Developer ID.
- `Credentials::load_app` answers `None`, the sign-in screen, when `account.json` has no login and the store has none to give or fails to read; the file stays and a new sign-in overwrites it. It moves a login an older app left in `account.json` into the store: it writes it, reads it back, and only then rewrites the file without it. A move that fails or stops halfway leaves the file's login in place for the next start.
- `Credentials::save_app` puts a new sign-in's login in the store and every other field in `account.json`. A store that refuses it leaves the login in the file.
- Signing out, or signing in over another login, removes `account.json` first, then the store's entry, then ends the login at the hub.
- Nodes are unchanged: they keep only the machine token (13.11).

## 14. The account's MCP servers (0.17.22)

Unit P3c, branch `parity/mcp`. The hub keeps a list of MCP servers per account, with an optional project scope. Every session pinned starts loads the enabled ones beside pinned's own server. The user manages the list in Settings › MCP servers, and agents manage it through `act`.

### 14.1 Data

`crates/core/src/mcp.rs`. A server follows the shape of Claude Code's own config, with the names of its secrets and never their values:

```json
{"id":"mcp-19a…","enabled":true,"server":{"name":"ctx","project":"github.com/acme/site","transport":{"type":"stdio","command":"npx","args":["-y","@acme/ctx"],"env":["API_KEY"]}}}
{"id":"mcp-19b…","enabled":true,"server":{"name":"linear","transport":{"type":"http","url":"https://mcp.linear.app/mcp","headers":[]}}}
```

- `mcp::Entry { id, enabled, server: Spec }`, `Spec { name, project?, transport: Reach }`, `Reach` tagged by `type`: `stdio { command, args, env }`, `http { url, headers }`, `sse { url, headers }`. `env` and `headers` are names.
- `project` is a repository as `repo_key` writes it (`github.com/owner/repo`), as tastes key projects. Absent means every project.
- `mcp::Secrets { env: {name: value}, headers: {name: value} }` carries the values on a save, and only there.
- `Spec::check` refuses a name outside `[A-Za-z0-9_-]{1,64}`, the name `pinned`, a URL that is not http or https, a bad env or header name, a name listed twice, and a command, argument or URL that holds a key-shaped string (`surface::mask_secrets`).

Why this shape. The transport is a tagged union, so a stdio server with a URL can't exist. It reads like `.mcp.json`, which agents know. Adding a transport is a new variant, and a new field on a variant takes `serde(default)`. Both are additive. The answer types are contract roots (`McpServer`, `McpLoad`).

### 14.2 Storage

On the hub, in the connector store, as the box Claude tokens are. The list is `Store.mcp_servers` (the `accounts.store` document), and each value is a sealed key `mcp:<id>:env:<NAME>` or `mcp:<id>:headers:<NAME>` (the `accounts.secrets` document, sealed by `Sealer`). Every write goes through `Storage::write_key`, so it never races the connectors' own key writes. A signed-out app's first import keeps the list.

### 14.3 Wire (`HubCall`, all additive)

| Call | From | Answer |
|---|---|---|
| `list_mcp_servers` | client, agent (read) | `Vec<mcp::Entry>` |
| `save_mcp_server { id?, server, secrets? }` | client, agent (confirm, asks `secrets`) | `mcp::Entry`. With `id` it changes that server and keeps its enabled state. A secret left out or blank keeps its value. A new name needs one: `<name> needs a value for <NAME>, which the user types in the app`. A name the server loses loses its value. |
| `enable_mcp_server { id, enabled }` | client, agent (write) | `mcp::Entry` |
| `remove_mcp_server { id }` | client, agent (confirm) | `null`. Its keys go too. |
| `import_mcp_server { machine, account, name, path?, project?, secrets? }` | client, agent (confirm, asks `secrets`) | `mcp::Entry`. The hub reads the server from that machine with `Call::McpImportable` and saves it. |
| `load_mcp_servers` | a machine only. A client gets `only a machine loads the account's MCP servers into the sessions it starts`. Excluded for agents. | `Vec<mcp::Load>`: the enabled servers with their secrets. |

`Call::McpImportable { account?, path? }` (excluded for agents) answers `Vec<mcp::Found { scope, server }>`: the servers a Claude Code account's `.claude.json` (and `path`'s `.mcp.json`) define, with command, arguments and URL in full and only the names of their env and headers. `mcp_servers` (P3b) stays the agents' read of those.

### 14.4 Agents

`save_mcp_server` and `import_mcp_server` ask for `secrets`. The user types each value in the app's form, which sends them in `decide_confirmation`'s `entered` by path: `{"secrets.env.API_KEY": "…"}`. `surface::with_entered` places a dotted path inside its asked object. `with_blank_asks` leaves an asked object to its default.

Approving needs the app unlocked (13.11), as every decision does. The user's own `save_mcp_server`, `import_mcp_server` and `remove_mcp_server` need it too on a locked login, since `surface::owner_only` reads their `confirm` class; `list_mcp_servers` and `enable_mcp_server` don't.

`secrets` is a secret name (13.7). An agent that sends any text inside it is refused (`an agent never sends a password, key or token: secrets`), and summaries and answers show it as `[hidden]`.

### 14.5 Sessions

At each session's start (`launch_tab` and a headless child's spawn) the machine asks the hub `load_mcp_servers`, waiting `MCP_WAIT` (3 s), and keeps the answer in memory only. When the hub does not answer, the session gets the list the hub last handed this process. The first session with no list waits that long for a link that is still coming up, as a node's restored sessions start before the link connects. Later ones never wait.

`mcp::for_session` picks the servers for every project and those for the session's repository; a project's server replaces an account-wide one of its name. A name the session's own account already configures is left out, so it stays the account's own: Claude reads user, local and project scope with P3b's reader, and Codex reads its `config.toml`'s `mcp_servers` tables.

`mcp::SessionFiles::write` writes `servers.json`, in Claude Code's `mcpServers` shape with the secrets in place, at mode 0600 in a 0700 folder `$TMPDIR/pinned-mcp-<pid>-XXXX`. The tab keeps the folder until its session ends or is replaced, and a headless child keeps it until its output ends. Dropping it deletes it. A folder whose pid no longer runs is swept at the next write.

- Claude Code: `--mcp-config <pinned's> <servers.json>`.
- Codex: `-c mcp_servers.<name>.command=<pinned>` with `args=["mcp-run", <file>, <name>]` for a program, and `url` with `http_headers_helper="<pinned> mcp-headers <file> <name>"` for http. `pinned mcp-run` execs the program with its env from the file, and `pinned mcp-headers` prints its headers as JSON. No secret reaches Codex's argv or the environment its shell commands inherit. Codex speaks streamable HTTP only, so an sse server stays out of Codex sessions.
- Cursor: `cursor-agent` takes no MCP flag. It reads only `~/.cursor/mcp.json` and the project's `.cursor/mcp.json`, which are the user's own files, so Cursor sessions get none of the list.
- Box workers run a pinned node linked to the hub, so their sessions load the list the same way.

### 14.6 OAuth

Claude Code keeps an MCP server's OAuth login in the same secure store as the Claude login. The store is one per config dir: the keychain item `Claude Code-credentials-<sha256(CLAUDE_CONFIG_DIR)[:8]>` (plain `Claude Code-credentials` for `~/.claude`), or `<dir>/.credentials.json`. Each login sits under `mcpOAuth["<name>|<sha256({type,url,headers})[:16]>"]` (seen in 2.1.322). Copying a login between accounts would edit Claude Code's private store and hand one rotating refresh token to several refreshers. So the page marks an http or sse server that sends no header with "If it signs in with OAuth, log in once per Claude account: run /mcp in a session of each account."

## PR

https://github.com/FelipeGazapina/pinned/pull/84

### 13.12 Run now (0.17.23)

Issue #127. A saved workflow runs now, from Settings › Workflows or from an agent's `run_workflow`, with no Linear issue or Slack thread behind it.

| Wire | From | Answer |
|---|---|---|
| `HubCall::RunWorkflow { id }` | a signed-in client; an agent through `act` | `RunStarted { workflow, machine, machine_name, tab }` (outbound root), or the refusal. Another account's id reads as `this account has no workflow <id>`. |
| `Call::RunNow { run: RunSpec, title }` | the hub only (excluded as `run_now`) | The new tab's id, with claude running. Refused past the limits. |

**What a Run now opens.** `Connectors::run_now(id, by)` builds the session a triggered run would open: the workflow's model, effort, permission mode, worktree (`--worktree=run-now-<ms>`), account, folder, repo and machine. A graph workflow walks from its trigger's outgoing edges, as a run does, to its first `talk` or `session` step, whose setup and text the session takes; a `call` step hands the walk to the called workflow, which then owns the session. On the way, steps that act on the trigger's issue or thread (`stage-picked`, `move-column`, `reply`, `page`, `file-issue`, `take-issue`) have nothing to act on and are passed by their `out` port, and so is a classify or switch that has one. A classify that branches only on its answers, and a `stop`, refuse: `workflow "<name>" branches at "<step>" on what its trigger found, and Run now has no Linear issue or Slack thread to read, so it can't pick a branch`. The prompt is the instructions, the proof and box notes the workflow would add, then `Run now by <who>, at <YYYY-MM-DD HH:MM UTC>.` and a note that `pinned stage`, `pinned slack reply` and `pinned linear` have nothing to act on, then the step's text. `<who>` is `the account's owner, from Settings › Workflows`, or `the Captain "<title>" through run_workflow` (the caller's role and title). The tab is named `<workflow> · Run now`.

**Trigger kinds.** `linear-label` and `slack-mention` run with no subject. `slack-message` and `called` are refused, since neither has a session to start without a thread: `workflow "<name>" sorts new Slack messages into other workflows and has no session of its own, so Run now has nothing to start`, and `workflow "<name>" answers only the Slack threads another workflow hands it, so Run now has no thread to give it`. A disabled workflow still runs now: the switch turns its trigger off, and a Run now is asked for by hand.

**Subject.** The session's environment carries `PINNED_CONNECTOR` and a subject `Subject::LinearIssue { issue: "run-now:<workflow>:<ms>" }` (`connectors::is_run_now`). It names no issue: it keeps one run apart from the next, so a machine that gets the same run again after a lost answer answers with the session it started, and the hub's box and placement paths work unchanged. `pinned stage` answers `Run now started this session, which has no Linear issue to move`.

**Machine choice.** The user's Run now goes through `Hub::open_placed`, the path a triggered run takes: the workflow's machine when online, else the account's other machines (most recently connected first), or its Upstash Box. The hub sends `Call::RunNow` instead of `Call::Run` for a Run now subject (`hub::run_call`), on the pool and to a box. Under the `ask` box permission the user's Run now click is the yes, since nothing would run it again after a yes in Connectors; `off` still refuses. A machine whose pinned predates `run_now` reads as `<name> runs an older pinned that has no Run now: update it in Settings › Machines`. The signed-out desktop app runs it on its own machine through the Tauri command `run_workflow`.

**Limits and metrics.** `Machine::run_now` opens the tab, then holds the run as a task under the limits of the project it opens in (`captain_scope` of its folder): `Limits::admit` on the session's `--model` and `--effort`, then `Delegates::claim` under workers-at-once, with the time limit and budget set as `start_claude`'s are. A refusal closes the tab and answers the limit's own text (the same as `delegate_task`'s). The tab is the task's thread (`runs: true`); its first finished turn ends the task, also for a tab with no worker role, and the task's `TaskRecord` goes to the hub's metrics, titled `<workflow> · Run now`. For the user the task's parent is its own tab (`delegator.kind` `session`, no `tab`), and a task whose parent is the tab that shows it is never told its own end (`registry::announces`).

**Agents.** `run_workflow` is `write` with `starts_work`, and in `HUB_LIMITS`. It is write, not confirm, for the reason `start_claude`, `enqueue` and `open_box` are: the limits hold the work it starts (tier, effort, workers at once, time, budget), every run is audited, and a confirm would put the owner in the loop for each run an agent is trusted to start within those limits. `surface::limited` runs it:

1. It builds the run as above, with `<who>` the caller.
2. The run starts only on the caller's own machine, as every `starts_work` act: a workflow set to another machine or to an Upstash Box answers `run_workflow starts work only on this session's own machine for now, and this workflow runs on <machine | an Upstash Box>`.
3. A workflow whose session skips permission prompts is refused for a caller whose own session asks for them: `run_workflow would run a session that skips its permission prompts, and your own session asks for them`. The machine checks it again (`the workflow's session runs with full access, wider than your session's auto`).
4. The hub sends `Call::ActFor { caller, call: RunNow }` to the origin machine. `act_for` runs it with the caller's limits (`limits_in`, narrowed by the caller's own task); the task's parent is the caller, which gets its notice, and its record names the caller as delegator.
5. The audit row is `done`, or `failed` with the refusal.

**App.** Each card in Settings › Workflows has a Run now button between its text and its switch. While the call runs it reads `Starting…`, disabled. Then a line under the card says `Started on <machine>. Open session` (which reveals the tab), or `Didn't start: <the refusal>`. A `slack-message` or `called` workflow, or one with an unknown trigger, shows the button disabled with the reason as its tooltip.

**Not like a triggered run.** A Run now opens the first session only. Graph steps after it (more talks, replies, issue moves) need the trigger's subject and are not walked, and `{{trigger.*}}` spans in its text stay as written. Triggered runs themselves still open with `Call::Run`, outside the delegation limits and metrics, as before.
