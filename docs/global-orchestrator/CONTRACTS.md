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

## 1. Identity of the two roles

| Role | How a tab is marked | Where it runs | `TabView` |
|---|---|---|---|
| Global orchestrator | env `PINNED_ROLE=global-orchestrator` (`global_orchestrator::ROLE`, `global_orchestrator::env()`, `is_global(env)`) | `~/.pinned/orquestrador` (`global_orchestrator::FOLDER` under home). Made on open. Never a listed project. | `orchestrator_global: true`, `role: null` |
| Capitão | env `PINNED_ROLE=orchestrator` + `PINNED_PROJECT=<path>` (existing `Role::Orchestrator`) | the project folder | `role: "orchestrator"` |

- There is no new `Role` variant. An older app shows the global tab as a plain thread.
- One global tab per machine: `open_global_orchestrator` returns the lowest-numbered tab with the global env, else adds one, not started.
- The global folder's memory store is the global memory. `project_of_tab(global tab)` answers the folder, so `memory_*` tools and automatic recall use it. `known_project(folder)` accepts it, so `Call::MemorySearch/Save/Forget/Status { path: <folder> }` work. `Call::ListProjects` never lists it.
- In tests the folder is `<config dir>/home/.pinned/orquestrador` (`test_binary()`), so tests never touch the real home.
- The global tab gets no `--append-system-prompt` yet. `role_of` is `None` for it, so `launch_tab` adds nothing. The node-local unit adds its prompt there.

## 2. Settings

`global_orchestrator::Settings`, schema name `GlobalOrchestratorSettings`. One file per machine, `<config dir>/global-orchestrator.json`, like `orchestration.json`.

```json
{ "name": "Orquestrador", "host": false, "busy": "queue", "reports": "auto", "group_reports": true, "read_aloud": false }
```

| Field | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | `"Orquestrador"` | Chat name and message prefix. UI options: Orquestrador, Almirante, Comandante. `cleaned()` trims it, drops `[` `]` and control characters, caps it at 40 characters, and puts the default back when blank. |
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

- None of them is `may_wait`. They run on the machine's in-order call queue.
- An older node answers any of them with `this machine runs pinned <v>, which can't answer that: unknown variant ...`. `pinned_core::link::is_unknown_command(err)` is true for it. The orchestrator tells Felipe that machine needs an update.

## 4. Hub calls (`HubCall`, `crates/core/src/wire.rs`)

| Call | JSON | Reply |
|---|---|---|
| `Relay` | `{"cmd":"relay","request_id":"r-1","to":{RelayTo}}` | `Handoff` from the target machine |
| `Overview` | `{"cmd":"overview"}` | `Overview` |

`RelayTo` is tagged by `kind`:

```json
{"kind":"captain","machine":"<machine id>","path":"/w/site","text":"Ship it.","sender":"Orquestrador","busy":"queue"}
{"kind":"orchestrator","report":{CaptainReport}}
```

- `captain` direction: the hub must take it only from the machine that hosts the account's global orchestrator, then call `Call::Enqueue { path, text, request_id, sender, busy }` on `machine`.
- `orchestrator` direction: the hub must take it only from a machine whose tab `report.tab` is a Capitão (`role == orchestrator`) with `cwd == report.path`. It fills `report.machine` and `report.machine_name` from the sending socket, finds the host machine, and calls `Call::OrchestratorReport { request_id, report }` there.
- How the hub finds the host is the hub-bridge unit's choice. `Call::GlobalOrchestratorSettings` on each online machine answers `host`.
- Today the hub refuses both, from a machine (`http.rs` `machine_frame`, which routes them to `hub_call`) and from a client (`hub.rs` `hub_call`), with `hub::not_yet(call)`:
  `this hub runs pinned <v>, which can't answer that yet: the global orchestrator's relay is not on the hub`.
  `is_unknown_command` is true for it and for an older hub's `this hub runs pinned <v>, which can't answer that: unknown variant ...`. Test: `crates/server/tests/hub.rs` `a_node_reads_the_hubs_refusal_of_the_global_orchestrator_calls_as_an_unknown_command`.
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
  }]
}
```

- `key` is `repo` when the project has an origin, else `<machine id>:<path>`. Tools take it as `project`.
- `captain` is `CaptainState`: `"livre"` (idle, done, failed or stopped: a message types now or starts it; also needs-input with `interrupted` false, which is Claude Code's idle notification after a finished turn), `"trabalhando"` (running with status working, or needs-input with `interrupted` true, a question mid-turn: a message queues), `"nenhum"` (no Capitão tab, or the machine is offline). `CaptainState::of(Option<&TabView>)` computes it. Use it on the hub and the node alike.
- `WaitingKind` is `"needs_input"` or `"failed"`. Adding a kind, such as a ready PR, is an additive enum change.

## 6. Session tools

The per-session MCP server (`crates/core/src/reminders.rs`) offers tools by `Seat`, which it reads from `PINNED_ROLE`. A delegated child (`PINNED_TASK` set) has no seat.

| Seat | Tools |
|---|---|
| Global orchestrator | reminders (when on) + `memory_search`, `memory_save`, `memory_forget`, `projects`, `send_to_captain`, `read_captain`, `waiting`. No delegation tools, no `delegate`, no `agents`. |
| Capitão | everything a plain thread sees + `tell_orchestrator` |
| Anyone else | exactly what they saw before |

`permissions()` lists all ten project tools, so each is pre-allowed. `Machine::project_tool` enforces the same rules, because the socket takes any tool name:

- `projects`, `send_to_captain`, `read_captain`, `waiting` from another tab: `only the global orchestrator has this tool`.
- `delegate`, `agents` from the global tab: `the global orchestrator does no project work: hand it to the project's Capitão with send_to_captain`.
- `tell_orchestrator` from a non-Capitão: `only a project's Capitão reports to the global orchestrator`.
- `Message::Delegate` from the global tab (`start_delegate`, the `delegate_task` path) is refused with the same text as `delegate`, so no headless child runs in the global folder.

| Tool | Arguments | Result (tool text) |
|---|---|---|
| `projects` | none | `Overview` as one line of JSON |
| `send_to_captain` | `project` (key from `projects`, or a folder on this machine), `message`, `request_id` (new per message; repeats are safe), `machine` (optional id or name; pass it when `projects` lists the project on more than one machine) | `Handoff` as one line of JSON |
| `read_captain` | `project`, `machine` (optional) | text with the latest replies |
| `waiting` | none | `Vec<Waiting>` as one line of JSON |
| `tell_orchestrator` | `text` | `Handoff` as one line of JSON |

`ProjectTool` variants (`crates/core/src/agent.rs`): `Projects`, `SendToCaptain { project, message, request_id, machine: Option }`, `ReadCaptain { project, machine: Option }`, `Waiting`, `TellOrchestrator { text }`.

`send_to_captain` resolution today (`send_to_captain`, `local_project`): when `machine` is absent or names this machine (`is_this_machine`), and `project` is a known project folder, `<this machine>:<path>`, or the `repo` of a local project, the message goes to `enqueue_to_captain` here. Otherwise it goes to `route_remote`.

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

**Message to a Capitão.** `[<sender>] <text>`, for example `[Orquestrador] Ship the form.` The sender is the cleaned settings name, so it has no brackets.

**Report to the global orchestrator** (`CaptainReport::line`):

```
[pinned] Capitão <project> (<machine name>) [<machine id> <tab id>]: <text>
```

- The project and machine names have `(`, `)`, `[`, `]`, `:` and control characters replaced by spaces. An unknown machine id is `-`. The text may span lines.
- UI regex for the header: `/^\[pinned\] Capitão ([^()\[\]\n]+) \(([^()\[\]\n]*)\) \[(\S+) (\S+)\]: /m`. The machine id and tab id link to the thread (`machine` + `tab`).
- One typed prompt may hold several reports, joined by the queue with `\n`. Each report starts at a line that matches the header and runs to the next header line or the end.

**Quick replies.** The orchestrator's prompt (node-local unit) ends a reply that asks Felipe something with a last line:

```
Opções: Sim | Ainda não
```

- UI: when the last non-empty line of an assistant message matches `/^Opções: (.+)$/`, split the capture on ` | `, hide the line, and render one button per option. A click types that option's text as the user's message.

## 9. Seams and the unit that fills each

All in `crates/core/src/machine.rs` unless noted. Today each returns the error shown, or does nothing.

| Seam | Signature | Today | Fills it |
|---|---|---|---|
| `projects_overview` | `fn projects_overview(&self) -> Result<global::Overview, String>` | `projects is not available yet` | node-local: hub `Overview` when linked, else this node alone. Calls the hub through the hub-bridge handle. |
| `waiting` | `fn waiting(&self) -> Result<Vec<global::Waiting>, String>` | `waiting is not available yet` | node-local. A needs-input tab with `interrupted` false is Claude Code's idle notification, not a question: leave it out. |
| `read_captain` | `fn read_captain(&self, project: &str, machine: Option<&str>) -> Result<String, String>` | `read_captain is not available yet` | node-local for this node. Reuse the hub MCP's `read_orchestrator` logic (`crates/server/src/mcp.rs`). hub-bridge for another machine. |
| `route_remote` | `fn route_remote(&self, project: &str, machine: Option<&str>, text: &str, request_id: &str, sender: &str) -> Result<Handoff, String>` | `<project> is not a project on this machine, and sending to a Capitão on another machine is not available yet` | hub-bridge: resolve the checkout from `Overview`, send `HubCall::Relay { to: captain }`, and turn `is_unknown_command` into "that machine or the hub needs an update". |
| `report_up` | `fn report_up(&self, captain: &str, project: &Path, text: String, answers: Vec<String>) -> Result<Handoff, String>` | `reporting to the global orchestrator is not available yet` | node-local: build `CaptainReport`; `deliver_report` when `settings.host`, else `HubCall::Relay { to: orchestrator }` (hub-bridge handle). |
| `captain_turn_ended` | `fn captain_turn_ended(&self, captain: &str, project: &Path, event: &AgentEvent)` | no-op. `record_event` calls it on Done or Failed of a Capitão, before the tab's queue drains. | node-local: under `ReportPolicy::Auto`, `self.sent.lock().unwrap().answer(project)`, read the last reply as `report_worker` does, `report_up(.., answers)`. |
| `is_this_machine` | `fn is_this_machine(&self, machine: &str) -> bool` | hostname match only | hub-bridge: remember the id from `ToMachine::Hello` and match it too. |
| hub handle | none yet | | hub-bridge: give `Machine` a way to call the hub, for example `set_hub(Arc<dyn Fn(HubCall) -> Result<Value, String> + Send + Sync>)` installed in `link::spawn`, like `set_sync_waker`. |
| `hub::not_yet` arms | `crates/server/src/hub.rs` `hub_call` | refuse | hub-bridge: implement `Relay` and `Overview`, with the source checks in `crates/server/src/http.rs` `machine_frame`. Extract the MCP's `message_orchestrator` code into a shared function. |
| global prompt | `launch_tab` | none | node-local: when `global::is_global(env)`, append the global orchestrator's prompt (routing rules, `#projeto` and `@máquina` tags, the quick-reply line). Add a Capitão prompt line about `[<name>]` messages and `tell_orchestrator`. |

## 10. Files each later unit mainly touches

| Unit | Files |
|---|---|
| node-local | `crates/core/src/machine.rs` (global prompt in `launch_tab`, `projects_overview`, `waiting`, `read_captain` local, `report_up`, `captain_turn_ended`), `crates/core/src/agent.rs` (prompts), `crates/core/src/global_orchestrator.rs` (prompt text, overview builders) |
| hub-bridge | `crates/server/src/http.rs`, `crates/server/src/hub.rs`, `crates/server/src/mcp.rs`, `crates/core/src/link.rs`, the remote seams in `machine.rs` (`route_remote`, `is_this_machine`, the hub handle), `crates/server/tests/hub.rs` |
| UI-chat | `ui/gui-sidebar.js` (fixed Orquestrador row from `orchestrator_global`, "Esperando você (N)"), the timeline files (report grouping by the header in section 8, quick-reply buttons) |
| UI-settings | the settings page (`ui/gui-orchestration.js` or a new file: `Call::GlobalOrchestratorSettings` / `SetGlobalOrchestratorSettings` on each machine, one `host`), the projects page, the memory panel (`ui/gui-surfaces.js`: `Call::Memory*` with `path` = the global tab's `cwd`) |

`machine.rs` is shared by node-local and hub-bridge. Each fills only its own seams to keep merges clean.

## 11. Rules every unit keeps

- Wire: add variants or `#[serde(default)]` fields only. After a wire change run `snapshot current`, then `check --base origin/main` must list no New breaks. Plain `check` exits 1 on main already, on old 0.8 to 0.12 breaks filed under Already shipped.
- Never type into a busy Capitão. Use `enqueue_to_captain` or `hand_to`; never call `type_prompt` on a Capitão directly.
- Keep the lock order `sent` then `tabs`.
- Keep request ids idempotent end to end. The hub passes the orchestrator's `request_id` through `Relay` to `Enqueue` unchanged.

## 12. Known gaps

- A Capitão running with no conversation that the user is looking at is relaunched with the message as its first prompt. Nothing is lost, because it has no conversation, but its screen restarts.
- When a Capitão tab is closed while a message waits in its queue, the message stays queued for a tab that is gone until a later restart rebinds it by session. The ledger keeps it `queued`.
- Without a running global tab on the host, a report starts it with the report as the first prompt, and the global prompt applies only once node-local adds it.
- A Capitão that `hand_to` starts with the message as its first prompt (`iniciado`) keeps that message only in its launch arguments until its first hook gives it a Claude session. When pinned quits in that window, for example while a folder-trust dialog waits, the tab is not saved, the message is lost, and the ledger still answers `iniciado` for its `request_id`. A message queued in the same window (`queue_request` with no session) stays bound to the dead tab id, like the closed-tab case above.
- A stopped Capitão restored with `Continue from where you left off.` as its launch prompt loses that prompt when `hand_to` starts it on a message. Nothing is lost from the conversation. The node starts restored tabs at once, so the window is short.
- `local_project` prefers this machine when `machine` is absent. The global prompt must tell the orchestrator to pass `machine` when `projects` lists more than one checkout, per the routing rule (running Capitão, then own machine, then first online).
