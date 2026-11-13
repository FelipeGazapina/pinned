# Ideas and Tasks — design (0.18.2)

Status: draft, design only. Base: `main` = v0.18.1 (`5af27835`). No code changes in this PR.

Felipe asked for two features:

- **Ideas.** A note store, so an idea is not forgotten: save, list, read, edit, delete. No scheduling, no execution.
- **Tasks.** "At 9 a.m. tomorrow, release app version x.x.x." Something an agent does at a time, once or on a repeat. Any agent can be the target, and any agent can do what the user can.

Both are reachable from the Orchestrator, from any session through `act`/`actions`, and from the UI (desktop, hub web, phone width).

**Naming.** The UI says **Ideas** and **Tasks**. On the wire and in agent tools, Tasks are named `save_schedule`, `list_schedules`, `pause_schedule`, `delete_schedule`, `run_schedule_now` (and Ideas `save_idea`, `list_ideas`, `delete_idea`), never `task_*`. `delegate_task`, `task_status`, `task_cancel` and `task_review` already exist for delegated child tasks (`crates/core/src/reminders.rs`, `crates/core/src/delegate/`), the composer already shows a "Tasks" strip for Claude Code's todo list (`ui/gui-composer.js:835`), and Usage shows "Tasks" for delegated runs (`ui/delegation.js:69`). An agent told to "create a task" must not end up calling `delegate_task`, so the tool names stay separate and the tool descriptions say which is which.

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
    #[serde(default)] pub grace_min: Option<u32>,     // 3.3; None = 15
    #[serde(default)] pub pickup_min: Option<u32>,    // 3.4; None = 10
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
    pub state: RunState,      // due | delivered | queued | started | stalled | held | done | failed | missed | skipped | lost
    #[serde(default)] pub delivered_at: Option<i64>,  // 3.4: the pickup clock starts here
    #[serde(default)] pub last_activity: Option<i64>, // 3.4: last time the target's transcript moved
    pub machine: String, pub tab: String, pub detail: String,
}
```

Rules:

- **"Agent type must not matter"** holds because the target is an address, not a role. Every delivery path below already exists for that kind of agent. The task module only picks the path.
- The scheduler lives **on the hub**, the one place that is always on, sees every machine, and already runs the connector loop. Tasks are stored in Postgres like connectors: tables `tasks`, `task_runs`, `ideas`.
- Natural language ("tomorrow at 9 a.m.") is turned into a `Schedule` by the agent, never by the hub. `save_schedule` validates the schedule and answers the next three run times in the task's zone ("next: Fri 13 Nov 09:00 BRT"), so the agent can read them back to Felipe before he moves on.
- `tz` defaults to the zone of the device or machine that saves the task (`Intl.DateTimeFormat().resolvedOptions().timeZone` in the UI, the machine's zone for an agent). The hub evaluates with `chrono-tz`. A local time skipped by a DST change runs at the first valid minute after it. A local time that happens twice runs once, at the first.

### 2.3 What is migrated or deleted

| Today | 0.18.2 | Later |
|---|---|---|
| `reminders::Reminders`, `reminders.json`, `watch_reminders`, `notify` (desktop-only) | On first start after update, a signed-in app turns each pending `Reminder` into a `Once` task with `Target::Me` and renames the file to `reminders.migrated.json`. `Target::Me` fires as a hub push to every device (desktop notification + web push via `sw.js`/`sw-notifications.js`) | Delete `Reminders`, `watch_reminders` and the file handling in 0.19 |
| `remind_me`, `list_reminders`, `cancel_reminder` tools, `agent::Message::{Remind, ListReminders, CancelReminder}` | Kept as **aliases** with the same names and arguments. They now create, list and cancel `Target::Me` tasks through the hub. Their descriptions say "prefer save_schedule". Sessions started by older apps keep working because the tool names do not change | Remove the aliases one minor version after every snapshot-listed app ships the schedule actions (the agent tool list is not wire-contract, but old MCP configs on disk still name the tools) |
| `--no-reminders` / `NO_REMINDERS` on nodes | `Target::Me` no longer needs a desktop, so nodes offer the tools too. The flag stays accepted and becomes a no-op | Delete the flag |
| Run now (`run_workflow`, `HubCall::RunWorkflow`, `Call::RunNow`) | Unchanged. `run_schedule_now` of an `Action::Workflow` task calls the same `Hub::run_workflow` | — |
| "Remember to do X someday" in `memory_save` | The Orchestrator's prompt (`global_orchestrator.rs:802`) says: facts go to memory, things to do someday go to `save_idea`. There is no automatic move of old memories: we cannot tell an idea from a fact without guessing. The Ideas page has "Import from Orchestrator memory…", which lists memories whose `kind` is `idea`/`todo` and lets Felipe pick them | — |

### 2.4 Ideas

- **Account-wide, with an optional project.** An idea is Felipe's, not a repo's: many ideas ("an iPad app", "a cheaper box tier") have no project yet. `project: Option<String>` (a repo key such as `github.com/FelipeGazapina/pinned`) lets the list filter by project and lets a Captain list "ideas for my project".
- **Type** (`crates/core/src/ideas.rs`, new): `Idea { id, title, text, project: Option<String>, status: Open | Done | Dropped, task: Option<String>, created_by: Actor, created_at, updated_at }`. Stored in the hub table `ideas`.
- **How an idea differs from memory.** Memory is *what is true*: preferences, decisions, conventions. Agents recall it into their prompts on their own. An idea is *what might be done*. It is never recalled into a prompt and is read only when someone asks. The tool descriptions say this in one line each. `save_idea` saves whatever it is given, and the Orchestrator prompt carries the split ("anota essa ideia" → `save_idea`; "lembra que eu prefiro X" → `memory_save`). A filter that guessed would only cause surprises.
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

---

## 3. Where and when a task runs

### 3.1 The clock is on the hub

- One loop per hub, `schedule::Scheduler`, beside `Connectors::watch` and started by `Hub::watch_all`. It sleeps until the earliest `next_at` across accounts, or 60 s at most, and wakes early on any save (a `Notify`, like `Connectors::wake`).
- At due time it **claims** the occurrence in one statement: `insert into task_runs (task, due_at, state) values ($1, $2, 'due') on conflict (task, due_at) do nothing returning ...`. Then it advances `tasks.next_at`. An occurrence is claimed exactly once, even if two hub processes overlap during a Railway deploy.
- Every delivery carries `request_id = "task:<task id>:<due_at>"`. `send_to_captain` and `enqueue` already deduplicate on it (the Captain request ledger, `LEDGER_FILE`). `prompt`, `tell_orchestrator`, `delegate_task` (`clientRequestId`) and `Target::Me` pushes are given the same key. A redelivery after a crash never acts twice.

### 3.2 Which machine, per target

| Target | Path at due time | Busy | Stopped / closed | Machine offline |
|---|---|---|---|---|
| `Me` | Hub push to every device: desktop notification, web push, and an Inbox row | — | — | No machine needed |
| `Orchestrator` | The machine that hosts it (`global_orchestrator::Settings.host`), typed into its tab like a Captain report | Queued behind its current turn (Orchestrator queue, same as `BusyPolicy::Queue`) | Its tab is opened and started on the prompt | Run is `queued`; delivered when the host comes back, within the grace window (3.3) |
| `Captain { project, machine }` | `send_to_captain`'s handoff: `Delivered`, `Queued`, `Started` | `Queued` behind its work; `Busy` (policy refuse) is retried every minute until grace ends | `Started` with the prompt (held to the project's tier, effort and worker limits) | If `fallback = box` and the box permission for Captains allows it, `send_to_captain { box: true }` opens it in a box. Otherwise the run waits within grace |
| `Session { session, machine }` | `prompt` into the tab that holds that Claude session | Waits for its turn to end (the hub watches `TabView.status`), then types | Archived → `unarchive_session` + `start_claude`, then the prompt. Closed and not archived → reopened by `open_run` on its machine | Waits within grace. A session cannot move machines, so there is no box fallback |
| `Worker { project, model }` | `delegate_task` placement (`placement.rs`, the account's prefer/avoid policy) | — (a new worker) | — | Another online machine with the project, else a box when `fallback = box`, else waits within grace |
| `Action::Workflow` | `Hub::run_workflow`; the workflow's `RunsOn` decides (`Any` / `Machine` / `FreshBox`) | — | — | `RunsOn::FreshBox` needs no machine; otherwise waits within grace |

- **No machine online at all.** `Me` and box-placed runs still go. Everything else waits. When the grace window ends, the run is `missed` and `on_missed` applies (3.3). The scheduler never invents a box for a target that did not opt in: a box costs money and clones a repo, so `fallback: Fallback::{Wait, Box}` defaults to `Wait`, and `Box` is offered only for `Captain`, `Worker` and workflow runs.
- **Limits.** A task run is work an agent started, so it goes through the same limits as Run now and `delegate_task` (`starts_work`): workers at once, tier, effort, time limit, budget. A refusal for a limit retries until grace ends and then is `failed` with the refusal text. It never bypasses a limit.
- **What the target sees.** The prompt starts with one line: `Scheduled task "Release pinned 0.18.2" (created by you / by the Captain of pinned, 12 Nov) [task:<id>:<due_at>]. Nobody is watching this run; steps that need Felipe will wait for him.` Then the text. The bracketed `request_id` sits inside the first 400 characters on purpose: that is what the hooks keep of a prompt (3.4). The session knows it runs unattended.
- **Busy is a routing hint, never proof.** The Busy column above only picks how the message is handed over. Whether the target actually took it is decided by 3.4, from signals that do not come from pinned's own busy flag.
- **Result.** A run is `done` when the target's first turn after pickup (3.4) ends. The hub reads the last reply (`read_session`) into `TaskRun.detail`, and the Tasks page links to the session. A one-time task then disables itself. It stays listed until deleted, so its history is not lost.

### 3.3 Missed runs and hub restarts

- **On time** means delivered within the task's grace window: 15 minutes by default, set per task.
- `OnMissed`, per task:
  - `Ask` — **default for `Once`.** The run becomes `missed`, and an Inbox / Needs you row reads "Release pinned 0.18.2 was due 09:00 and did not run (pinned-mac was offline). **Run now** · **Skip**". A one-time action like a release should not happen hours late without Felipe saying so.
  - `Skip` — **default for `Cron`.** The run is recorded as `skipped`, and the next occurrence comes as usual. A daily digest at 09:00 should not also run at 15:00.
  - `RunLate` — run as soon as a path opens, whenever that is.
- **Never a burst.** Missed occurrences are coalesced: after a week offline, a daily task with `RunLate` runs once, not seven times. Older occurrences are recorded `skipped`.
- **Hub restart / deploy.** State is all in Postgres (`tasks.next_at`, `task_runs`). On boot the scheduler loads every enabled task and treats each occurrence with `due_at < now` and no row as missed (via `on_missed`). Rows left in `due` or `delivered` stay as they are. Because the `request_id` is stable, the target's dedupe ledger answers "already have it". A delivery whose answer was lost (`Unanswered::Lost`) is marked `lost`, exactly as a workflow run is, and is **never** resent to a second machine. The Tasks page shows it with "Run now" for Felipe to decide.
- **The task changed while due.** Editing a task deletes its unclaimed future and recomputes `next_at`. Runs already claimed keep the version they started with: the run row stores a copy of the action.

### 3.4 Pickup: a delivered run that never starts

**Why.** On 12 Nov the Captain of pinned showed `trabalhando` from 17:38 to 22:25 after its turn had ended, and everything sent to it sat in its queue (draft PR #228, `.claude/research/captain-queue-stuck/REPORT.md` on `fix/captain-queue-stuck`). A Stop hook hand-over timed out on the hook side after 300 ms, but the machine had already set `Tab::turn_pending` and marked the message in flight, so `notice_idle()` stayed false and `CaptainState::of` answered `Working`. #228 fixes that path. This design must not depend on it. A task run handed to a target that pinned *thinks* is busy is `Queued` behind that belief (3.2), and any similar bug would silently eat scheduled runs. So a run is only `started` when the hub sees the target take the message, from a signal that pinned's busy state does not produce.

**Signals, today and new** (line numbers on `main` at 5af27835):

| Signal | Where it exists | Counts as pickup? | New in PR 5 |
|---|---|---|---|
| `TabView.status` / `CaptainState::of` (`Working` when `orchestrator_turn` or status `Working`) | `global_orchestrator.rs:405`, `machine.rs:549` | **No.** This is the busy flag that lied for five hours. Used only to choose Queued vs typed | — |
| Ledger `Progress::InFlight` | `global_orchestrator.rs:155`, set by `Ledger::typed` (`:215`) from `mark_typed` (`machine.rs:5622`) | **No.** Set when pinned *hands the text over*, before Claude Code has it. In the bug it was `InFlight` the whole time | — |
| Ledger `Progress::Answered` | set when a Captain turn ends, `captain_turn_ended` (`machine.rs:3254`) | Proof of a finished turn, so it implies pickup. Too late to be the pickup signal | — |
| UserPromptSubmit hook → `AgentEvent { status: Working, prompt }` | `hooks.rs:204` keeps the prompt cut to 400 chars (`history::clip_prompt`, `history.rs:190`); hook name in `agent.rs:223`; handled by `record_event` (`machine.rs:5903`) | **Yes**, when the prompt holds the run's `[task:<id>:<due_at>]`. Claude Code fires it only when a turn really starts from that text | The machine matches the marker against the runs it was handed and sends `ScheduleEvent::PickedUp { request_id, at }` to the hub |
| The session transcript (`.jsonl` Claude Code writes) | `Machine::transcript` (`machine.rs:1390`) → `find_transcript` (`machine.rs:1462`) | **Yes**, when a `user` line or a `hook_additional_context` attachment after `delivered_at` holds the marker. Written by Claude Code, not by pinned. This also covers the hand-over path, where the text arrives as Stop/UserPromptSubmit `additionalContext` (`hooks.rs:231`) and no prompt event carries it | A tail scan from the last offset, every 30 s for each run in `delivered`/`queued`/`stalled`, and the file's size/mtime kept as `last_activity` |
| A delegated worker's first stream event | `delegates.turn_started(task)` (`machine.rs:5924` → `delegate/registry.rs:402`) | **Yes** for `Target::Worker` | Map the task id back to the run |
| `Target::Me`, `Action::Workflow` | — | `Me` needs no pickup (a push). A workflow has its own run states; pickup applies to its first step's session | — |

**The pickup deadline.** `Task.pickup_min`, default 10 minutes, set per task like grace (`grace_min`). It is stored, not a constant, so a task that targets a Captain known for long turns can carry 60. The clock starts at `delivered_at`, which is when the handoff answered `Delivered`, `Queued` or `Started`. When it passes with no pickup signal, the run goes to **`stalled`**. Then:

- Nothing is delivered anywhere else. No second machine, no box, no new tab. The queued copy stays where it is, so a late pickup still works.
- Felipe decides, from a row (below). A pickup that comes after `stalled` (someone typed "read your queue") moves the run to `started` and closes the row with "started late, 22:25".
- **Skip** withdraws the queued copy from the target's queue (new `Call::WithdrawRequest { request_id }`; the ledger marks it answered without typing), so it does not fire hours later as hook context. If the target already took it, the answer says so and the run follows the pickup.
- **Deliver again** goes to the same target on the same machine only. It first withdraws the queued copy, then types the prompt into the tab whatever pinned's busy flag says (a new `BusyPolicy::TypeNow`; Claude Code queues typed text itself while a real turn runs, so typing is safe), with `request_id = task:<id>:<due_at>:again<n>`. There is never more than one live copy.
- **Open** opens the tab (`open_run`) so Felipe can look.

**Busy that is real but long.** A 2-hour turn and a stuck flag look the same to the busy flag. They do not look the same in the transcript: a working turn writes a line for every tool call and every reply, so its file grows. At the deadline the hub compares `last_activity` with now:

- **Alive** (the transcript moved within the last `pickup_min`): the run stays `queued` (not `stalled`), and an FYI row goes to the Inbox only: "waiting behind a running turn, last activity 40 s ago". No push. It clears itself on pickup. If the wait outlives the grace window of a `Once` task, `on_missed` applies as in 3.3 (`Ask`: "still busy; run when it finishes, or skip?"), because a release three hours late is Felipe's call even when nothing is broken.
- **Silent** (the flag says working, the transcript has not moved for `pickup_min`): `stalled`, a Needs you row and a push. In the 12 Nov case the transcript was still from 17:38, so the row would have come at 17:48 instead of nobody knowing until 22:25.
- A turn that is waiting on a long tool (a 40-minute `cargo build` in the foreground) writes nothing while it waits. It looks silent. pinned does not guess: the row says what it saw, and Felipe opens the tab. A false alarm costs a look; a missed stall cost five hours.

**What Felipe sees.**

Desktop, Needs you (and the run's line on the Tasks page):

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ⏱  Release pinned 0.18.2                                         stalled     │
│ Delivered 09:00 to the Captain of pinned on pinned-mac. It has not started   │
│ after 10 min. pinned shows it working, but its conversation has not moved    │
│ since 08:41.                                                                 │
│                                       [ Open ]  [ Deliver again ]  [ Skip ]  │
└──────────────────────────────────────────────────────────────────────────────┘
```

Desktop, Inbox FYI (alive busy):

```
│ ⏱  Daily digest — waiting behind a running turn of the Captain of pinned    │
│    (turn running 1 h 52 min, last activity 40 s ago).        [ Open ] [ Skip ]│
```

Phone (≤ 768 px), Needs you card; buttons stack full width, Skip last:

```
┌────────────────────────────────┐
│ ⏱ Release pinned 0.18.2        │
│ stalled · 09:00 · pinned-mac   │
│ Delivered to the Captain of    │
│ pinned. Not started after      │
│ 10 min; no activity since 08:41│
│ ┌────────────────────────────┐ │
│ │            Open            │ │
│ ├────────────────────────────┤ │
│ │        Deliver again       │ │
│ ├────────────────────────────┤ │
│ │            Skip            │ │
│ └────────────────────────────┘ │
└────────────────────────────────┘
```

**A target that went away.** The machine goes offline after `Delivered`: the run is `lost` as in 3.3, never resent elsewhere. The tab closed before pickup: `stalled` with "the tab closed before it started"; Deliver again reopens it on the same machine.

---

## 4. Safety

A scheduled run has no one watching. The rule, as the Orchestrator proposes it to Felipe:

> **Anything that normally needs Felipe's OK stops at that step and waits for him, unless he approved that exact action when he created the task.**

### 4.1 What "needs Felipe's OK" means, concretely

Two existing lists, and nothing new to invent:

1. **`act` actions of class `Confirm`** (`surface.rs`: `delete_session`, `remove_machine`, `update_self`, `disconnect_*`, …). These already wait for the user's decision in the app, whether or not anyone is watching. No change, except that the confirmation carries the task id, so Needs you can say "from scheduled task Release pinned 0.18.2".
2. **The never list** (`preauth::never::NeverItem`): `DeleteOutside`, `ForcePush`, `PushToMain`, `Release`, `Deploy`, `Production`, `Secrets`, `Payment`, `WriteOutside`, `SelfGrant`, `NetworkInstall`, `MachineWide`. Release and deploy are shell commands (`gh release create`, `git push origin v*`, `railway up`, `vercel --prod`), not `act` calls, so this list is the only thing that catches them. It already parses Bash lines word by word and leans toward a hit when in doubt.

### 4.2 The held step

- While a session works on a task run (`PINNED_SCHEDULE=<task>:<due_at>` in its environment, the way `PINNED_TASK` marks a delegated task), its PreToolUse hook (`hooks.rs`; the Orchestrator's `GUARDED` hook today, extended to every session pinned starts for a task) calls `never()` on each Bash, Write and Edit.
- **A hit** that no approval covers: the hook answers `deny` with the reason `held for Felipe: release (gh release create v0.18.2) — pinned asked him; wait for the notice, do not work around it`. The machine creates a `Confirmation` with `held: Held { task, item: NeverItem, command }`, and the run goes to `held`. Felipe gets a push notification and a Needs you row: **Approve** · **Deny**.
- **Approve** records a one-shot allowance for that exact normalized command in that session and sends the existing `notice` ("approved: run it again now"). The hook lets that one command through once.
- **Deny** sends "denied: stop and report". After 24 hours with no decision, the run is `failed` ("held step expired"), and the session is told to stop.
- This is the same "show the normal prompt and record that the ask was held for Felipe" that `never.rs` was written for (DESIGN-preauth-approvals). A task run is its first caller.
- **The bypass setting does not change this.** A Captain that normally runs with permissions skipped still has its never-list hits held during a task run. The never list applies to the run, not to the session's own permission mode.

### 4.3 Approving at creation

- `Task.approvals: Vec<Approval { item: NeverItem, must_contain: String, repo: String }>`. Example: `{ item: Release, must_contain: "v0.18.2", repo: "github.com/FelipeGazapina/pinned" }`. A held command is covered only when its item matches, it runs in that repo's checkout, and its text contains `must_contain`. That is "that exact action": releasing **v0.18.2** of pinned, not "any release". An empty `must_contain` is refused.
- **Only Felipe grants approvals.** In the UI he ticks "Allowed without asking: ☑ release v0.18.2" in the task editor. When an agent sends `save_schedule` with `approvals`, the save is classified `Confirm`: it becomes a confirmation that shows the task and each approval, and nothing is saved until he approves. A `save_schedule` without approvals is `Write`.
- An approval never covers `Secrets`, `SelfGrant` or `Payment`. Those always hold. It also never covers what AGENTS.md reserves for the owner: approving the `hub-breaking-change` environment, or deleting or editing a wire snapshot.
- Approvals are shown on the task card ("✓ may release v0.18.2"). Changing a task's action or target drops its approvals, and Felipe has to grant them again.

### 4.4 Agent-created tasks

- `created_by: Actor { role: ActorRole, machine, tab, session_title }`, or the user. Every schedule and idea act goes through `act`, so the existing audit trail (`agent_actions`) already records who, when and with which arguments. Acts inside a task run carry `AuditEntry.task`.
- **Visible.** A task created by an agent shows a chip ("by Captain · pinned") on its card. Creating one sends an FYI row to the Inbox: "The Orchestrator scheduled *Release pinned 0.18.2* for Fri 13 Nov 09:00 — Open · Pause". There is no approval for a plain prompt task: an agent that can `prompt` or `send_to_captain` now can already do the same thing now, and scheduling adds no power, only a delay.
- **Guard rails on agents** (not on Felipe):
  - minimum interval 15 minutes for an agent's cron;
  - at most 20 enabled agent-created tasks per account (`Refused` with the list);
  - a task run may create tasks, but one created inside a task run cannot itself create more (`depth ≤ 1`, carried in the environment), so a task cannot keep rescheduling itself;
  - an agent may edit, pause or delete any task, as the user can. Felipe asked for that, and the audit trail shows it. Deleting a task Felipe created is `Confirm`.
- **One switch.** "Pause all tasks" on the Tasks page and as `pause_schedule { all: true }` (that one is `Confirm` for agents: a stuck agent must not silence everything).

---

## 5. UI

Both features are **shell pages** (`SHELL_PAGES` in `ui/gui-shell.js`). They open beside the sidebar like Needs you, Pull requests and Usage, not inside Settings, because they are daily work and not setup. The hub web app serves the same `ui/`, so desktop and web are one implementation. The cards reuse the Workflows look (`.flow-card`, `.new-card`, `.switch`, `.run-now-outcome`; `ui/style.css:279`, `ui/gui-settings.css:58-66`), so no new visual language is needed.

### 5.1 Sidebar (`ui/index.html` `#side-nav`, drawn by `drawNav` in `ui/gui-sidebar.js`)

Two `nav-row`s go between Needs you and Pull requests. `count` and `meta` follow the existing rows: Tasks shows the next run time in `meta` and a `count` of runs waiting on Felipe (held or missed). Ideas shows the number of open ideas in `meta`.

```
┌ sidebar ─────────────────────────────┐
│ ◉ Orchestrator        pinned-mac   2 │
│ ▢ Needs you                        3 │
│ ⏱ Tasks               next 09:00   1 │  ← new: count = held/missed runs
│ ✎ Ideas                       12 open│  ← new
│ ⇅ Pull requests            2 to review│
│ ▤ Usage               5h 41% ▓▓░░░░  │
│ ──────────────────────────────────── │
│ [project ▾]                       ＋ │
│ threads…                             │
```

Held and missed runs also appear on **Needs you** (`gui-needs-you.js`) as rows with a "Scheduled task" chip and their buttons. Felipe acts on them where he already looks.

### 5.2 Tasks page

```
┌ Tasks ──────────────────────────────────────────────── [Pause all] ┐
│ A task has an agent do something at a time: once, or on a repeat. │
│                                                                   │
│ ┌───────────────────────────────────────────────────────────────┐ │
│ │ ⏱ Release pinned 0.18.2                     [Run now]  (●on)  │ │
│ │   Once · Fri 13 Nov 09:00 BRT · Captain of pinned             │ │
│ │   ✓ may release v0.18.2   · by the Orchestrator               │ │
│ └───────────────────────────────────────────────────────────────┘ │
│ ┌───────────────────────────────────────────────────────────────┐ │
│ │ ⟳ Morning PR digest                         [Run now]  (●on)  │ │
│ │   Weekdays 08:30 · Orchestrator · last: done 08:31 ▸ Open     │ │
│ └───────────────────────────────────────────────────────────────┘ │
│ ┌───────────────────────────────────────────────────────────────┐ │
│ │ ⟳ Triage Slack bugs (workflow)              [Run now]  (○off) │ │
│ │   Every day 18:00 · runs workflow “Slack bugs” · paused       │ │
│ └───────────────────────────────────────────────────────────────┘ │
│ ┌ held ─────────────────────────────────────────────────────────┐ │
│ │ ⚠ Deploy docs — waits on you: deploy (vercel --prod)          │ │
│ │                                   [Approve] [Deny] ▸ Session  │ │
│ └───────────────────────────────────────────────────────────────┘ │
│ [＋ New task]                                                     │
│ Reminders (just notify me): 2 · show                              │
└───────────────────────────────────────────────────────────────────┘
```

- Card click opens the editor. Expanding a card's history lists its last 20 `task_runs` (state, machine, link to the session).
- `Target::Me` tasks (the old reminders) fold into a "Reminders" line at the bottom, so notification pings do not crowd out the agent work.

**Editor** (the page's own view, like the workflow editor's `view.name === "edit"`):

```
┌ ← New task ──────────────────────────────────────────────────────┐
│ Title    [Release pinned 0.18.2                               ]  │
│ Who      [Captain ▾] [pinned ▾]       (Me · Orchestrator ·       │
│                                         Captain · Session · New  │
│                                         worker · Workflow)       │
│ What     [Release the app as v0.18.2: bump, tag, changelog… ]   │
│ When     (•) Once   [2026-11-13] [09:00]                         │
│          ( ) Repeat [Weekdays ▾] [08:30]   ( ) Cron [0 9 * * 1] │
│          Time zone  America/Sao_Paulo ▾                          │
│          Next: Fri 13 Nov 09:00 · —  · —                         │
│ If missed  [Ask me ▾]   Grace [15] min   If offline [Wait ▾]     │
│ Allowed without asking (only you can tick these):                │
│          ☑ release   containing [v0.18.2]  in [pinned ▾]        │
│          ☐ deploy  ☐ push to main  ☐ delete outside worktree    │
│                                        [Delete]  [Cancel] [Save] │
└──────────────────────────────────────────────────────────────────┘
```

The "Next:" line comes from the hub's answer to a dry-run `SaveSchedule { dry_run: true }`, so the UI never re-implements cron or DST.

### 5.3 Ideas page

```
┌ Ideas ─────────────────────────── [All projects ▾] [Open ▾] [🔍 ] ┐
│ Things you might do. Agents read them only when asked; facts      │
│ you want agents to always know go in memory instead.              │
│ ┌ Quick idea… ───────────────────────────────────────── [Save] ┐ │
│ ┌───────────────────────────────────────────────────────────────┐ │
│ │ iPad layout for the sidebar                       pinned · 2d │ │
│ │ Split view with threads left, …           [Schedule…] [⋯]    │ │
│ └───────────────────────────────────────────────────────────────┘ │
│ ┌───────────────────────────────────────────────────────────────┐ │
│ │ Cheaper box tier                     — · by the Orchestrator  │ │
│ │ Scheduled: Fri 09:00 ▸                    [Schedule…] [⋯]    │ │
│ └───────────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────┘
```

- The quick-add field saves a title only. Opening an idea gives a title + text editor with autosave, the way the taste chat saves.
- The `⋯` menu holds: Edit, Send to Captain now, Mark done, Drop, Delete.

### 5.4 Phone width (≤ 768 px, `ui/gui-phone.js`)

The tab bar stays at four tabs: Threads, Needs you, Orchestrator, More. Tasks and Ideas are **the first rows of More's first list** (`MORE_LISTS[0]`), with hints from `moreHints` ("next 09:00 · 1 waits", "12 open"). Held and missed runs already reach Needs you, which has its own tab.

```
┌ More ───────────────────────┐
│ Usage  5h ▓▓▓░░ 41%          │
│ ┌─────────────────────────┐ │
│ │ Tasks     next 09:00 ›  │ │
│ │ Ideas        12 open ›  │ │
│ │ Pull requests 2 to rev ›│ │
│ │ Projects …            › │ │
│ └─────────────────────────┘ │
├─────────────────────────────┤
│ Threads  Needs you  Orch  More│
└─────────────────────────────┘

┌ ← Tasks          [＋]       ┐   cards go full width; Run now and the
│ ⏱ Release pinned 0.18.2     │   switch wrap under the title; the
│   Once · Fri 09:00 · Captain│   editor's rows stack one per line;
│   [Run now]          (●on)  │   date/time use native inputs.
│ ⟳ Morning PR digest         │
│   Weekdays 08:30 · Orch     │
│   [Run now]          (●on)  │
└─────────────────────────────┘
```

- On the phone, Ideas' quick-add sits pinned above the keyboard, so capturing an idea is one tap from More.
- The Orchestrator tab is the other quick path: "anota: iPad layout" → `save_idea`.

### 5.5 Workflows page

Each `flow-card` (`ui/settings.js` `cardHtml`) gets a line under its sentence, "⏱ Weekdays 09:00 · next Mon 09:00", when tasks run it. The editor gets a **Schedule…** button beside **Run now**. It opens the task editor with `Who = Workflow` locked to this workflow. A workflow that cannot Run now (`runNowBlocked`) cannot be scheduled either, and shows the same reason.

---

## 6. Pull requests, in order

Each PR runs `cargo run -p pinned-server --bin wire-contract -- snapshot current` and `-- check --base origin/main`, and must show **no new breaks**. UI PRs add `ui/*.test.js` and a tutorial video (the `pinned:tutorial-video` skill: real clicks through the sidebar, desktop width and 390 px phone width).

| # | PR | Contents | Proof | Model |
|---|---|---|---|---|
| 1 | `core: schedule and idea types` | `crates/core/src/schedule.rs` (`Task`, `Schedule`, `Target`, `Action`, `OnMissed`, `Fallback`, `Approval`, `TaskRun`, `next_after(schedule, tz, after)`), `ideas.rs` (`Idea`). Deps: `chrono-tz`, `croner` (cron with DST-safe evaluation). No callers yet | Unit tests: once / cron / weekdays; DST gap (São Paulo, New York) and overlap; leap day; "tomorrow 09:00" across midnight; invalid cron refused with a readable error; serde round-trip with defaults (an older JSON without the new fields) | **sonnet** |
| 2 | `hub: ideas store and calls` | Tables `ideas`; `HubCall::{ListIdeas, SaveIdea, DeleteIdea}`; `Settings.ideas` push field; `surface::ACTIONS` rows `list_ideas`/`save_idea`/`delete_idea` (Read/Write/Write, wired); the idea tools for the Orchestrator and every session; Orchestrator prompt split memory vs idea (`global_orchestrator.rs:802`) | `crates/server/tests/hub.rs`: save, list by project, edit, delete, another account cannot see; `surface.rs` tests (`every_wire_call_has_one_action`); wire-contract check additive | **sonnet** |
| 3 | `ui: Ideas page` | Shell page `ideas`, sidebar nav row, More row on phone, quick-add, editor, filters | `ui/ideas.test.js` (render, filters, quick-add sends `save_idea`); tutorial video: sidebar → Ideas → add → edit → delete, then the same at phone width via More | **sonnet** |
| 4 | `hub: schedule store and calls` | Tables `tasks`, `task_runs` (`unique (task, due_at)`); `HubCall::{ListSchedules, SaveSchedule{dry_run}, PauseSchedule, DeleteSchedule, ScheduleRuns}`; `Settings.schedules`; actions `list_schedules` (Read), `save_schedule` (Write; **Confirm** when it carries approvals, through a `.checks()` like `marks::agent_check`), `pause_schedule`, `delete_schedule` (Confirm for a user-created task); agent caps (15-min interval, 20 tasks) | Hub tests: CRUD; dry-run answers next 3 times in tz; agent with approvals → confirmation, not saved; caps refused; audit rows carry the creator | **sonnet** |
| 5 | `hub: the scheduler` | `schedule::Scheduler` loop in `Hub::watch_all`; claim-once insert; delivery per target (`Me` push, `Orchestrator`, `Captain` via the `send_to_captain` handoff, `Session` via `prompt` with wait-for-idle / unarchive, `Worker` via delegate placement, `Workflow` via `run_workflow`); `request_id = task:<id>:<due>`; grace, `OnMissed`, coalescing; restart recovery; `RunScheduleNow` + `run_schedule_now` action (`starts_work`); pickup (3.4): the `[task:…]` marker in the first line, `ScheduleEvent::PickedUp` from the UserPromptSubmit prompt and from a transcript tail scan, `last_activity`, `pickup_min`, `stalled`, `Call::WithdrawRequest`, `BusyPolicy::TypeNow` for Deliver again | Hub tests with a paused tokio clock: fires once; two schedulers on one DB fire once; host offline → waits → delivered in grace / missed after; `Ask` creates a Needs-you row; `Skip` and coalescing after a 7-day gap; hub restart mid-run does not resend (`Lost`); busy Captain → queued; limits refusal → retried then failed. **Pickup (3.4), machine + hub:** (a) *stuck flag*: Captain tab `status = Done` with `turn_pending` set and the ledger `InFlight` (so `CaptainState::of` = `Working`), handoff `Queued`, transcript unchanged → at +10 min the run is `stalled`, one Needs-you row with the 3.4 text, one push, and exactly one delivery recorded on any machine; (b) ledger `InFlight` alone never counts as pickup; (c) *alive busy*: status `Working`, transcript appended every minute → at +10 min still `queued`, Inbox FYI row, no push; Done + next prompt with the marker → `started`, row closed; (d) pickup through a UserPromptSubmit prompt; pickup through a Stop-hook hand-over seen only in the transcript; (e) late pickup after `stalled` → `started`, row says "started late"; (f) Deliver again withdraws the queued copy, types once on the same machine, a later Stop hook hands over nothing; (g) Skip withdraws the queued copy; (h) `pickup_min = 2` honored, missing field = 10 (serde default); (i) `Once` task alive-busy past grace → `Ask` row; (j) `Worker` with no `turn_started` in time → `stalled` | **opus** |
| 6 | `safety: held steps in task runs` | `PINNED_SCHEDULE` env; PreToolUse guard on task-run sessions calling `preauth::never`; `Confirmation.held` (`serde(default)`); `HubCall::DecideHeld` via the existing decision path; one-shot allowance; approvals match (item + repo + `must_contain`); 24 h expiry; never-coverable items | Unit tests on the matcher (exact version only; Secrets never covered); hook tests (`hooks.rs` style: hit → deny + confirmation; approved → passes once; second time held again); hub test end to end with a fake session; wire check | **opus** |
| 7 | `ui: Tasks page` | Shell page `tasks`, nav row with next run and held count, cards, editor (dry-run Next line, tz picker, approvals only in the user's UI), history, Pause all; Needs you rows for held and missed; More row on phone | `ui/tasks.test.js` (card sentences, editor builds `Schedule`, held row buttons); tutorial video: create "release at 09:00" task with an approval, Run now, see it held for an unapproved deploy, approve from Needs you; phone width via More | **sonnet** |
| 8 | `ui+hub: workflows ↔ tasks, ideas → tasks` | Schedule… on workflow cards/editor, "⏱ next" line; Schedule… on ideas (`task.idea`), idea `Done` when its one-time task finishes | UI tests; hub test idea status follows its task; tutorial video: schedule a workflow, schedule an idea | **sonnet** |
| 9 | `reminders → tasks` | `remind_me`/`list_reminders`/`cancel_reminder` become aliases over `Target::Me` tasks; `reminders.json` migration on first signed-in start; nodes offer them; `Target::Me` push to desktop + web push | `crates/core/tests/delegate_mcp.rs` updated: same tool names and arguments, now land on the hub; migration test (file → tasks → `.migrated`); desktop notification still shows (manual note in PR) | **sonnet** |
| 10 | `docs + changelog` | CHANGELOG 0.18.2 entries, Orchestrator help text, `docs/global-orchestrator/CONTRACTS.md` section for the schedule and idea actions | Review only | **haiku** |
| 11 | *(0.19)* `remove reminders store` | Delete `Reminders`, `watch_reminders`, `--no-reminders` handling; later the alias tools | Tests still green; wire check | **haiku** |

Order and parallelism: 1 → (2, 4) in parallel → 3 after 2 → 5 after 4 → 6 after 5 → 7 after 5 (held UI after 6) → 8, 9 → 10. Ideas (1–3) can ship alone if Tasks slip.

---

## Open questions for Felipe

1. Default grace window: 15 minutes. Long enough for a laptop that is waking up?
2. When an agent creates a task, should it need his OK (a Confirm), or is the FYI row enough? This design says FYI, because scheduling gives an agent no power it does not already have.
3. Should a `Once` task's `Ask` row also send a push notification at the missed time, or only show in Needs you?
4. Ideas: should a project's Captain see its project's ideas in its opening context, or only on request? This design says on request only, to keep ideas apart from memory.
