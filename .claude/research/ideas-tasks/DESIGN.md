# Ideas and Tasks — design (0.18.2)

Status: draft, design only. Base: `main` = v0.18.1 (`5af27835`). No code changes in this PR.

Felipe asked for two features:

- **Ideas.** A note store, so an idea is not forgotten: save, list, read, edit, delete. No scheduling, no execution.
- **Tasks.** "At 9 a.m. tomorrow, release app version x.x.x." Something an agent does at a time, once or on a repeat. Any agent can be the target, and any agent can do what the user can.

Both are reachable from the Orchestrator, from any session through `act`/`actions`, and from the UI (desktop, hub web, phone width).

**Naming.** The UI says **Ideas** and **Tasks**. On the wire and in agent tools, Tasks are called `schedule_*`, never `task_*`. `delegate_task`, `task_status`, `task_cancel` and `task_review` already exist for delegated child tasks (`crates/core/src/reminders.rs`, `crates/core/src/delegate/`), the composer already shows a "Tasks" strip for Claude Code's todo list (`ui/gui-composer.js:835`), and Usage shows "Tasks" for delegated runs (`ui/delegation.js:69`). An agent told to "create a task" must not end up calling `delegate_task`, so the tool names stay separate and the tool descriptions say which is which.

---

## 1. What overlaps today

Three things in the tree touch "remember this" or "do this later". None of them is a scheduler that makes an agent act at a time.

### 1.1 Workflows (`Connector`)

- Type: `pinned_core::connectors::Connector` (`crates/core/src/connectors.rs:271`). Fields include `trigger: Trigger`, `folder`, `model`, `effort`, `permission_mode`, `worktree`, `instructions`, `runs_on: RunsOn` (wire name `machine`), `account`, `slack_bot`, and the optional `graph: Option<graph::Graph>`.
- Triggers are events only: `Trigger::{LinearLabel, SlackMention, SlackMessage, Called}` (`connectors.rs:109`), mirrored in the graph as `graph::Watch::{LinearLabel, SlackMention, SlackMessage, Called}` (`crates/core/src/graph/mod.rs:183`). **There is no time trigger.** The only clock is `connectors::INTERVAL` (60 s), which is how often `Connectors::watch` (`connectors.rs:3221`) polls Linear and Slack.
- Storage: hub Postgres, per account, through `PgStorage` (`crates/server/src/hub.rs:298`). The hub starts one watch loop per account with connectors (`Hub::watch_all`, `hub.rs:281`).
- **Run now:** `HubCall::RunWorkflow { id }` (`crates/core/src/wire.rs:1081`) → `Hub::run_workflow` (`hub.rs:367`) → `Connectors::run_now` (`connectors.rs:4859`) builds a `Run` with no issue or thread → `Hub::open_placed` sends it where `RunsOn` says (`Any`, `Machine(id)`, `FreshBox(BoxStep)`), and the machine answers through `Call::RunNow` (`wire.rs:274`). Agents reach it as the `run_workflow` action (`crates/core/src/surface.rs`, class `Write`, `starts_work`). The machine-side `run_now` is `Excluded`. Run now refuses `SlackMessage` and `Called` workflows, because they have no session of their own.
- UI: Settings › Workflows (`ui/settings.js:848` `connectorsHtml`, `cardHtml`, `runNowHtml`). Each `flow-card` has a step chain, a sentence, a **Run now** button and an on/off switch.

So a workflow is "**what** a session does and **where** it runs, started by an **event**". A scheduled run of a workflow is a Run now at a time. Nothing does that today.

### 1.2 The Orchestrator's reminders (`remind_me`)

- Store: `pinned_core::reminders::Reminders` (`crates/core/src/reminders.rs:35`), a `Vec<Reminder { id, message, due_unix_secs }>` in `<config>/reminders.json` **on the desktop machine only**.
- Tools: `remind_me`, `list_reminders`, `cancel_reminder` (`reminders.rs:15-17`, `reminder_tools()`), offered by the `pinned mcp` server (`reminders::serve_io`) only when the machine has a desktop. A node passes `--no-reminders` (`NO_REMINDERS`; `link.rs:901`, `self_update.rs:867`). Times come from `agent::When::{InMinutes(f64), At("HH:MM")}`, so it can only schedule within the next 24 hours, in the machine's local time.
- Firing: the Tauri app's `watch_reminders` (`src/main.rs:391`) polls every second and calls `notify`, which on macOS shows an `osascript` notification. The socket messages are `agent::Message::{Remind, ListReminders, CancelReminder}` (`crates/core/src/agent.rs:401-406`, handled at `src/main.rs:628`).
- Limits: a reminder only notifies. It never wakes an agent, never reaches the phone or hub web, is lost if the app is closed at its time (`take_due` fires late, on the next start), and the other machines and the hub cannot see it.

A reminder is a one-time task whose target is "Felipe", not an agent.

### 1.3 Orchestrator memory (`memory_save`)

- Store: `pinned_core::memory::{Store, Memories, Memory { id, title, text, kind, created, updated }}` (`crates/core/src/memory.rs`). One SQLite database per project folder, with full-text plus vector search (model2vec).
- The global Orchestrator runs in `~/.pinned/orquestrador` (`global_orchestrator::FOLDER`), so "that folder's memory is the account's global memory" (`global_orchestrator.rs:3`). Its prompt tells it to save durable facts about Felipe with `memory_save` (`global_orchestrator.rs:802`).
- Actions: `memory_search`, `memory_save`, `memory_forget`, `memory_status` (`surface.rs:171-174`).
- `Memories::recall` (`memory.rs:427`) **injects matching memories into prompts**. Memory is context that steers agents automatically.

Today, "Felipe, remember to try X someday" ends up in `memory_save` because nothing else exists. Memory is the wrong place for it: the fact gets recalled into unrelated prompts as if it were a rule, it sits on one machine's disk, and Felipe has no list to browse.

### 1.4 Related pieces Tasks will reuse

| Piece | Where | What Tasks takes from it |
|---|---|---|
| `send_to_captain` and its request ledger (`captain-requests.json`, `LEDGER_FILE`) | `global_orchestrator.rs`, tool in `reminders.rs` | Deliver to a Captain: opens or starts it, queues while busy, and the same `request_id` never delivers twice |
| `enqueue`, `prompt`, `tell_orchestrator` | `surface.rs:161,181`, `reminders.rs:676` | Deliver to a Captain, a running session, or the Orchestrator |
| `delegate_task` + `placement.rs` | `delegate/`, `placement.rs` | Start a fresh worker on the best machine or a box |
| Confirmations + audit | `surface::{Confirmation, AuditEntry, ActorRole}`, tables `agent_confirmations`, `agent_actions` (`crates/server/src/db.rs:98-111`) | Hold a dangerous step for Felipe; record who created or changed a task |
| The never list | `crates/core/src/preauth/never.rs` (`NeverItem::{Release, Deploy, DeleteOutside, ForcePush, PushToMain, Production, …}`) | Decide which steps of an unattended run wait for Felipe. It exists but nothing calls it yet (slice 1a of #159) |
| The Orchestrator's PreToolUse guard | `hooks.rs:45`, `agent::Message::Guard` | The hook point where a held step is stopped |
| Box placement | `RunsOn::FreshBox`, `send_to_captain { box }`, `crates/server/src/boxes.rs` | Run when no machine is online |

---

## 2. The model

### 2.1 Recommendation

**Tasks are the account's one scheduler.** A task is *when*, *who* and *what*. Running a workflow is one kind of *what*. A reminder is one kind of *who*. **Workflows stay what they are**: event-triggered graphs. They do not get a time trigger.

```
Task  = Schedule  (once, cron, in a time zone)
      × Target    (me | orchestrator | captain | session | new worker)
      × Action    (prompt text | run workflow)
      + policy    (missed runs, approvals, where to fall back)
```

Why not a `Watch::Schedule` trigger on workflows, making Tasks "workflows with a clock"?

1. **Targets.** A workflow run always opens its own new session (`Connectors::run_now` builds a `Run`). Felipe's main case, "Orchestrator, release at 9", targets an agent that already exists: the Orchestrator, a project's Captain, or a session he is talking to. A workflow graph has no node for "type into an existing agent", and adding one would change the `Run` model that 4,700 lines of `connectors.rs` assume.
2. **Wire.** A new `Watch` or `Trigger` variant shows up inside `Connector`, which every app version reads through `Settings.connectors`. `project.rs` exists only to give older apps a lossy legacy view of graphs. A new trigger kind means more lossy projection. A separate `tasks` list that older apps never receive costs nothing.
3. **"Tasks are workflows, but more generic"** is exactly the relation here: a task can run a workflow (`Action::Workflow`), so every workflow can run as a task, and a task can do things no workflow can (prompt the Orchestrator, notify Felipe).

The links go both ways:

- **Task → workflow:** `Action::Workflow { id }` runs the same path as Run now (`Hub::run_workflow`), with the workflow's own `RunsOn`, limits and proof rules.
- **Workflow → tasks:** the workflow card and editor list "Scheduled: every weekday 09:00" and get a **Schedule…** button that creates a task with `Action::Workflow`. A workflow's schedule *is* a task. There is no second schedule store.
- **Workflow step → task (later, optional):** a graph node `Schedule { in, prompt }` that creates a follow-up task ("check this PR again in 2 hours"). It is additive (a new `NodeKind` variant that only new graphs carry) and is not part of 0.18.2.

### 2.2 Types (in `crates/core/src/schedule.rs`, new)

```rust
pub struct Task {
    pub id: String,                 // uuid
    pub title: String,              // "Release pinned 0.18.2"
    pub schedule: Schedule,
    pub tz: String,                 // IANA, as "America/Sao_Paulo"; set from the creator's device
    pub target: Target,
    pub action: Action,
    pub enabled: bool,              // false = paused
    #[serde(default)] pub on_missed: OnMissed,
    #[serde(default)] pub approvals: Vec<Approval>,   // section 4
    #[serde(default)] pub fallback: Fallback,         // section 3
    #[serde(default)] pub idea: Option<String>,       // the idea it came from
    pub created_by: Actor,          // user, or role + machine + tab + session title
    pub created_at: i64, pub updated_at: i64,
    // Hub-computed, read-only:
    #[serde(default)] pub next_at: Option<i64>,
    #[serde(default)] pub last_run: Option<TaskRun>,
}

#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Schedule {
    /// A wall-clock date and time in `tz`: "2026-11-13T09:00".
    Once { at: String },
    /// Five-field cron, evaluated in `tz`. The UI's Daily, Weekdays, Weekly and Monthly presets write these.
    Cron { expr: String },
}

#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Target {
    /// Felipe himself: a notification on every device, with no agent run. This is what reminders become.
    Me,
    /// The global Orchestrator, on the machine that hosts it.
    Orchestrator,
    /// A project's Captain, by repo key; `machine` empty = wherever `send_to_captain` would pick.
    Captain { project: String, #[serde(default)] machine: String },
    /// An existing conversation, by Claude session id, so it survives tab restarts.
    Session { session: String, #[serde(default)] machine: String },
    /// A new worker in a project, placed like `delegate_task` (account placement policy, box allowed).
    Worker { project: String, #[serde(default)] model: String },
}

#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Action {
    /// Text the target receives as one prompt, signed "Scheduled task <title> (by <creator>)".
    Prompt { text: String },
    /// Run now of a saved workflow. The target is ignored: the workflow's own `runs_on` decides.
    Workflow { id: String },
}

pub struct TaskRun {          // one row per occurrence, table `task_runs`
    pub task: String, pub due_at: i64, pub started_at: Option<i64>,
    pub state: RunState,      // due | delivered | queued | held | done | failed | missed | skipped | lost
    pub machine: String, pub tab: String, pub detail: String,
}
```

Rules:

- **"Agent type must not matter"** holds because the target is an address, not a role. Every delivery path below already exists for that kind of agent. The task module only picks the path.
- The scheduler lives **on the hub**, the one place that is always on, sees every machine, and already runs the connector loop. Tasks are stored in Postgres like connectors: tables `tasks`, `task_runs`, `ideas`.
- Natural language ("tomorrow at 9 a.m.") is turned into a `Schedule` by the agent, never by the hub. `schedule_save` validates the schedule and answers the next three run times in the task's zone ("next: Fri 13 Nov 09:00 BRT"), so the agent can read them back to Felipe before he moves on.
- `tz` defaults to the zone of the device or machine that saves the task (`Intl.DateTimeFormat().resolvedOptions().timeZone` in the UI, the machine's zone for an agent). The hub evaluates with `chrono-tz`. A local time skipped by a DST change runs at the first valid minute after it. A local time that happens twice runs once, at the first.

### 2.3 What is migrated or deleted

| Today | 0.18.2 | Later |
|---|---|---|
| `reminders::Reminders`, `reminders.json`, `watch_reminders`, `notify` (desktop-only) | On first start after update, a signed-in app turns each pending `Reminder` into a `Once` task with `Target::Me` and renames the file to `reminders.migrated.json`. `Target::Me` fires as a hub push to every device (desktop notification + web push via `sw.js`/`sw-notifications.js`) | Delete `Reminders`, `watch_reminders` and the file handling in 0.19 |
| `remind_me`, `list_reminders`, `cancel_reminder` tools, `agent::Message::{Remind, ListReminders, CancelReminder}` | Kept as **aliases** with the same names and arguments. They now create, list and cancel `Target::Me` tasks through the hub. Their descriptions say "prefer schedule_save". Sessions started by older apps keep working because the tool names do not change | Remove the aliases one minor version after every snapshot-listed app ships `schedule_*` (the agent tool list is not wire-contract, but old MCP configs on disk still name the tools) |
| `--no-reminders` / `NO_REMINDERS` on nodes | `Target::Me` no longer needs a desktop, so nodes offer the tools too. The flag stays accepted and becomes a no-op | Delete the flag |
| Run now (`run_workflow`, `HubCall::RunWorkflow`, `Call::RunNow`) | Unchanged. `schedule_run_now` of an `Action::Workflow` task calls the same `Hub::run_workflow` | — |
| "Remember to do X someday" in `memory_save` | The Orchestrator's prompt (`global_orchestrator.rs:802`) says: facts go to memory, things to do someday go to `idea_save`. There is no automatic move of old memories: we cannot tell an idea from a fact without guessing. The Ideas page has "Import from Orchestrator memory…", which lists memories whose `kind` is `idea`/`todo` and lets Felipe pick them | — |

### 2.4 Ideas

- **Account-wide, with an optional project.** An idea is Felipe's, not a repo's: many ideas ("an iPad app", "a cheaper box tier") have no project yet. `project: Option<String>` (a repo key such as `github.com/FelipeGazapina/pinned`) lets the list filter by project and lets a Captain list "ideas for my project".
- **Type** (`crates/core/src/ideas.rs`, new): `Idea { id, title, text, project: Option<String>, status: Open | Done | Dropped, task: Option<String>, created_by: Actor, created_at, updated_at }`. Stored in the hub table `ideas`.
- **How an idea differs from memory.** Memory is *what is true*: preferences, decisions, conventions. Agents recall it into their prompts on their own. An idea is *what might be done*. It is never recalled into a prompt and is read only when someone asks. The tool descriptions say this in one line each. `idea_save` saves whatever it is given, and the Orchestrator prompt carries the split ("anota essa ideia" → `idea_save`; "lembra que eu prefiro X" → `memory_save`). A filter that guessed would only cause surprises.
- **Idea → task.** "Schedule…" on an idea opens the task editor with the title and text filled in and `task.idea = idea.id`. The idea stays, shows "Scheduled: Fri 09:00", and moves to `Done` when a one-time task's run ends `done`. "Send to Captain now" is the same as Run now: a task with no schedule is just `send_to_captain`, so it calls that directly and creates no task.

### 2.5 Wire contract (AGENTS.md)

Everything is additive:

- New `HubCall` variants: `ListIdeas`, `SaveIdea`, `DeleteIdea`, `ListSchedules`, `SaveSchedule`, `PauseSchedule { id, paused }`, `DeleteSchedule`, `RunScheduleNow`, `ScheduleRuns { id, limit }`, `DecideHeld` (section 4). Older apps never send them.
- New hub push data: `ideas` and `schedules` ride as **new `#[serde(default, skip_serializing_if = "Vec::is_empty")]` fields** of the existing settings push (`connectors::Settings`, `connectors.rs:1138`), the way `convex` and `box_asks` were added. An older app ignores them.
- `Connector` gets **no** new field. The workflow card finds its schedules by filtering `schedules` on `action.workflow == id`.
- `AuditEntry` gets `#[serde(default)] task: Option<String>` for acts done inside a task run. `ActorRole` gets **no** new variant: a task run acts as the target's own role. An older app that reads the audit trail would fail on an unknown role.
- `Confirmation` gets `#[serde(default)] held: Option<Held>` for a held step (section 4). Older apps show it as a plain confirmation with its `about` text.
- New `surface::ACTIONS` rows (they are not wire, but `every_wire_call_has_one_action` requires one per `HubCall`).
- No HTTP route is removed. Each PR runs `wire-contract -- snapshot current` and `-- check --base origin/main`.
