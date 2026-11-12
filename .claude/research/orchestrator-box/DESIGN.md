# Orchestrator on a temporary Upstash Box (0.18.2 design)

Status: design only. Base `origin/main` 5af27835 (v0.18.1).

The user has no computer online, or none they can use. From the hub's web page (a phone browser
is enough), they start the global Orchestrator on a temporary Upstash Box. The box starts with
everything the Orchestrator remembers, does what the Orchestrator does on a computer, and opens
Captains on their own boxes. Once it sits idle, it hands its memory back to the hub and is deleted.
The next Orchestrator, on a box or a machine, picks up where it stopped.

## 1. What exists today

Each item below names the code that shows it.

- The Orchestrator is one tab on one machine. Its folder is `~/.pinned/orquestrador`
  (`global_orchestrator::FOLDER`, `Machine::open_global_orchestrator`, machine.rs:2711). The tab
  is created unstarted and starts on its first message (`hand_to`, machine.rs:2915).
- The machine that hosts it is "the one online machine whose `global-orchestrator.json` says
  `host: true`". The hub asks every online machine each time (`relay::host`, `relay::the_host`).
  Zero hosts or two hosts is an error. Every host-only call goes through it: overview,
  `send_to_captain`, read calls, repo reads and Captain reports (`relay::only_host`).
- `send_to_captain` with `box: true` sends `RelayTo::CaptainBox`. The hub runs
  `Boxes::open_captain` (boxes.rs:908), which checks the Upstash key, the `orchestrator` box
  permission ("Captains"), the per-account Claude token (`token_kept`), the open-box cap and the
  Box budget. It writes a `boxes` row with `BoxWork::Captain` and boots in the background.
- A box boots as a node of the account (`boot_node`). The hub writes `account.json`, `box.env`
  (Claude token and GitHub token, mode 0600) and `start.sh` through the Box API. Then it runs
  `box-bootstrap.sh`, which carries no secret and downloads `pinned-server` from the hub.
- The reaper (`reap_once`) ends a running row through `verdict`. The limits are idle 10 min,
  exited 5 min, no tab 2 min, offline 10 min and max life 4 h, plus the budget. `finish`
  removes the machine, deletes the box and deletes the row. Every step is idempotent, so a hub
  restart finishes a half-ended row.
- Every linked node runs the sync loop every 20 s (`link.rs` `SYNC_EVERY`), box nodes included.
  It syncs each checkout with a git origin under the key `host/owner/name` (`Machine::checkouts`).
  It moves the Claude transcripts (`claude/*.jsonl`), the Claude file memory (`claude/memory/**`),
  untracked skills and pinned's turn history. The hub stores them in `project_files`.
  `pinned:plugins` is a precedent for a key that is not a repository (`sync::PLUGINS_KEY`).
- The Orchestrator's folder has no git origin, so **none of its files sync today**.
- Pinned memories (`memory_save`/`memory_search`/`memory_forget`) are one SQLite file per
  project folder, in `<config>/memory/<sha8(path)>.sqlite` (`memory::Memories::store`). The
  machine is the only writer. **Pinned memories never leave the machine today, for any project.**
- Reminders are kept and fired by the desktop app (`src/main.rs` `Reminders::take_due`). A node
  with no desktop has none.

## 2. Memory inventory

"Machine file" means it lives only on the host machine today. "Share" means the new sync key
`pinned:orchestrator` (section 4.1). Paths use `<home>` for the Claude home of the account the
Orchestrator runs under, `<slug>` for Claude Code's folder name of the Orchestrator folder, and
`<config>` for pinned's config folder.

| Part | Where it lives today | Hub-side or machine file | Sync path in (to the box) / out (to the hub) |
|---|---|---|---|
| Pinned memories (`memory_save`, `memory_search`, `memory_forget`, the per-prompt recall hook) | `<config>/memory/<sha8(folder)>.sqlite` on the host | Machine file. Never synced. | Out: each machine exports its rows to `pinned/memories/<machine id>.jsonl` in the share. In: each machine merges every `pinned/memories/*.jsonl` into its SQLite (4.2). |
| Claude Code file memory: `MEMORY.md` and the files it links | `<home>/projects/<slug>/memory/**` | Machine file. Not synced, because the folder has no origin. | Share path `claude/memory/**`, as projects sync it today. |
| Conversation transcripts (the chat) | `<home>/projects/<slug>/*.jsonl` | Machine file | Share path `claude/*.jsonl`. The next Orchestrator resumes the newest one. The hub can also read it with no machine online (4.5). |
| Pinned's turn history of the folder | `history::file_for(<history dir>, folder)` | Machine file | Share path `history.jsonl`. |
| Folder instructions: `CLAUDE.md`, `AGENTS.md`, `.claude/settings.json`, `.claude/skills/**` in `~/.pinned/orquestrador` | The Orchestrator folder (empty on this Mac today) | Machine file | Share paths `folder/CLAUDE.md`, `folder/AGENTS.md`, `folder/.claude/settings.json`, and `skills/**` for the skills. Nothing else under `folder/`. |
| Orchestrator settings: name, busy policy, reports, group reports, read aloud | `<config>/global-orchestrator.json` on every machine (the UI writes them all at once) | Machine file on each machine | Out: the host writes `pinned/settings.json` without `host`, and adds `account`, the agent account its tab runs under. In: the box writes its own `global-orchestrator.json` from it at boot. |
| System prompt (`global_orchestrator::prompt`) and its Claude settings: no Bash, file tools guarded (`hooks::write_global_settings`) | Built into the binary | Code | None. The box downloads the same `pinned-server` build from the hub. |
| The Orchestrator's way of working (taste brief, Orchestrator scope) | Hub, `accounts.tastes` | Hub-side | None needed. |
| Delegation stats and task records | Hub, `delegation_tasks` | Hub-side | None needed. |
| Audit trail and confirmations of `act` | Hub, `agent_actions` and `agent_confirmations` | Hub-side | None needed. |
| Thread names and pins | Hub, `session_marks` | Hub-side | None needed. |
| Repo insight switch, box permissions, Box key, Claude tokens | Hub, the account's connectors store and secrets | Hub-side | None needed. The key and tokens never enter the share. |
| Captain request ledger (`captain-requests.json`) | The Captain's machine | Machine file of the Captain, not the Orchestrator | None. Reports reach whichever machine holds the seat (section 5). |
| Reminders | The desktop app's `reminders.json` | Machine file of the desktop app | Cannot move (section 6). |
| Plugins and `~/.claude/skills` | `<home>/plugins`, `<home>/skills` | Machine files, synced already under `pinned:plugins` | None needed. |
| User-level `<home>/CLAUDE.md` | The account's Claude home | Machine file. Never synced. | Not carried. It applies to every session, not only the Orchestrator. Open question 5. |

## 3. Start flow

### 3.1 Where the user starts it

- **Offered.** The Orchestrator row in the sidebar (`drawNav`, gui-sidebar.js) and the phone tab
  (`gui-phone.js` screen `orchestrator`) stop going grey when no machine is online. A tap opens a
  sheet that says "No computer is online." It shows the last conversation read-only (4.5), when
  memory last reached the hub, the box price per hour and the Box budget, and one button:
  **Start on a temporary box**.
- **By hand.** The Orchestrator settings section (`globalOrchestratorSettingsHtml`) adds a
  **Temporary box** chip next to the machine chips under "Runs on". The Orchestrator chat's
  header menu gets **Move to a box** while a machine holds it, and **Move back to <machine>**
  while a box holds it.
- The sheet shows the reason up front when the box can't start: Upstash not connected, the
  Sessions box permission off, no Claude token for the account, the open-box cap reached. It
  reads the reason from `OrchestratorSeat.can_box` before the user taps.

### 3.2 Hub

`HubCall::OpenOrchestratorBox { size, budget_usd }` comes from a signed-in client only, and
`Boxes::open_orchestrator` runs. Every check runs before anything is billed. It follows
`open_session` and `open_captain`:

1. `box_keys()`, else `NOT_CONNECTED`.
2. Box permission `session`. Off refuses. Ask and Fresh go ahead, because the user's own tap is
   the yes, as with Open box. (Open question 1.)
3. Signs in as `account` from the share's `pinned/settings.json`, else `accounts::default_account()`.
   `token_kept` refuses with the existing text: "No Claude token is kept for <account>: add it in
   Settings › Integrations › Upstash Box › Claude token per account."
4. `within_cap`.
5. Size: the call's, else `Small`. It does not use the repo-based `box_size`, because the box
   clones no repo.
6. Budget: `asked`, at most the Box budget of `TasteScope::Orchestrator`. It needs
   `>= BOX_USD_PER_HOUR`, as `open_session` does.
7. The seat check (section 5): a running Orchestrator turn on the machine host refuses with
   "The Orchestrator on <machine> is in the middle of a turn; move it when it stops."
8. Writes the row: connector `pinned-orchestrator`, subject `LinearIssue { issue: "the Orchestrator" }`.
   The table's `unique (account, connector, subject)` makes a second start answer the same row.
   `work: BoxWork::Orchestrator`, `spec.args = ["--name", "Orchestrator"]`, `spec.repo = None`.
9. Boots in the background (`reserve_work`) and answers `BoxStatus` at once. The client polls
   `box_status`, as the add-project box flow does (gui-add-project.js:330), and shows
   creating, then booting, then running.

### 3.3 Box boot

The box boots as today (`boot` and `boot_node`), with two changes:

- `box.env` adds `PINNED_BOX_ROLE=orchestrator`, which is not a secret. The node reads it to know
  it serves the seat.
- `start.sh` trusts `~/.pinned/orquestrador` along with the clones.

Once the node is online, `dispatch` for `BoxWork::Orchestrator` sends **`Call::OpenGlobalOrchestrator`**
and then **`Call::FlushOrchestrator`**. The first call runs one sync cycle of the share, which
pulls the memory before the tab starts. Then `Call::StartClaude` starts the tab. When the share
holds a transcript, the tab resumes the newest one (4.4). The box node's sync loop already runs
every 20 s, so the share keeps moving after boot.

### 3.4 Session

The UI's `globalHostMachine()` (global-chat.js) and `openGlobalChat()` read the seat from
`OrchestratorSeat`. Opening the Orchestrator then reveals the box's global tab, as it reveals a
machine's. Messages, uploads, voice and the Memory panel (`openGlobalMemory`) reach the box
through the hub as they reach any machine.

### 3.5 Identity and placement

- The box is a machine of the account named **`Box · Orchestrator`**, with `upstash_box` set.
  The Upstash name is `orchestrator-<6 hex>` (`BoxRole::Orchestrator`).
- The Orchestrator tab on the box is the account's Orchestrator. Its folder is the box's
  `~/.pinned/orquestrador`, and it has `PINNED_ROLE=global-orchestrator`, as on a machine.
- `projects()` leaves out the Orchestrator's own box. Captain boxes stay listed, as today.

## 4. Memory: moving it both ways

### 4.1 The share `pinned:orchestrator`

- `Machine::orchestrator_share()` returns a checkout-like value. Its `key` is
  `pinned:orchestrator`, its `path` is `global_folder()`, its `claude_home` is the home of the
  account the folder runs under, and its `history` is the folder's history file.
  `Syncer::cycle` syncs it next to the checkouts and the plugin share. **Every 0.18.2 machine
  syncs it**, so a machine that later takes the seat already has a copy.
- `sync::Roots` gains the `folder/` root and the `pinned/` root for this key only.
- Hub side, `sync::accepts("pinned:orchestrator", path)` takes only these paths:
  - `claude/*.jsonl`
  - `claude/memory/**`
  - `history.jsonl`
  - `skills/**`
  - `folder/CLAUDE.md`, `folder/AGENTS.md`, `folder/.claude/settings.json`
  - `pinned/memories/<uuid>.jsonl`, `pinned/settings.json`

  The hub refuses every other path, as it refuses non-plugin paths under `pinned:plugins`.
  `blocked()` names (`.env`, keys) stay blocked.
- The 32 MB file limit holds. A transcript past it stays on its machine, as for projects.

### 4.2 Pinned memories: one file per writer, merged on read

The pinned memories are a SQLite database, so file sync cannot move them as they are. Two
machines writing one file would also lose rows whenever both changed it ("newer copy wins").
So each writer owns its own file. Nothing shares a file, and nothing needs a lock.

- Schema: `ALTER TABLE memories ADD COLUMN uid TEXT` (a UUID, filled for old rows on open), and
  a `forgotten(uid TEXT PRIMARY KEY, at INTEGER)` table.
- **Export.** After a save or a forget on the Orchestrator's store, the machine writes
  `pinned/memories/<its machine id>.jsonl`. The file is every row it knows, as
  `{uid, id, title, text, kind, created, updated}`, plus `{uid, forgotten_at}` lines.
  Embeddings stay out, because `embed_missing` rebuilds them. Then it wakes the sync loop
  (`set_sync_waker`), so a memory reaches the hub within seconds rather than at the next 20 s tick.
- **Import.** When a sync pulls any `pinned/memories/*.jsonl`, the machine merges all of them:
  - Union by `uid`. The newest `updated` wins.
  - A `forgotten` uid is removed.
  - A row keeps its `id` when that id is free locally. With one seat at a time, ids stay the same
    across machines, so a resumed chat's `[42]` still means memory 42. Only a true conflict
    (open question 4) gets a new id.
- Merging is commutative, so the order of pulls never matters, and a retry changes nothing.
- Growth: one file per machine that ever held the seat. Each file is a few KB per 100 memories.
  Compaction is a follow-up and is not needed for 0.18.2.

### 4.3 Claude file memory, transcripts, history and folder files

These are plain files and sync as projects sync them. A file changed on both sides keeps the
newer copy. The seat (section 5) keeps one writer at a time, which makes that rule safe here.

### 4.4 The conversation continues across moves

- When `open_global_orchestrator` makes a **new** tab and the local Claude folder for the
  Orchestrator holds synced transcripts, the tab's `claude_session` is set to the newest one.
  The first start then passes `resume_args` (`hand_to`, machine.rs:2930), so the box continues
  the same conversation.
- When a sync pulls a `claude/<id>.jsonl` that a **live** tab on this machine runs, that tab is
  marked stale. Its next message restarts it with `RestartClaude` before typing. So a MacBook that
  slept while the box talked never types into an old in-memory conversation.
- To be checked in PR 2: Claude Code resumes a transcript whose lines name another `cwd`. Pinned
  already moves a transcript between Claude homes on one machine (`Call::SwitchAccount`), but a
  move across folders has not been run. If it fails, the fallback is to start fresh. The chat
  stays readable through 4.5, and the memory still moves.

### 4.5 The chat stays readable with no machine online

`HubCall::OrchestratorHistory { before, limit }` answers a `ThreadPage` from the hub's copy of
the newest `claude/*.jsonl` under `pinned:orchestrator`. It uses the newest-first paging that
`Call::Thread` uses. The parsing of `thread::read_back` moves into a function that takes bytes, so
the hub and the node share it. The sheet in 3.1 shows that page read-only.

### 4.6 Flush before delete

- `Boxes::finish` gains one step for `BoxWork::Orchestrator` rows, after the row is Ending and
  before `end`: `Call::FlushOrchestrator` on the box, waiting up to 60 s. The node exports the
  memories, runs one sync cycle of the share, and answers `{pushed, pulled}`.
- Flush succeeds: `end` deletes the machine, the box and the row.
- Flush fails while the box is online: the row stays Ending, and the next reap tries again. After
  3 tries (about 3 min), it ends anyway and logs "memory written after <last push> may be lost".
- Box already offline or vanished: it ends at once. The last periodic push stands.

### 4.7 Crashes

- **Hub restarts mid-end.** The row is still Ending. `reap_once` calls `finish` again, and the
  flush runs again. It is idempotent.
- **Box dies or is deleted outside pinned.** What reached the hub survives:
  - pinned memories, pushed within seconds of each save (4.2)
  - the file memory, transcripts and history, pushed within 20 s

  A `PostToolUse` hook on `Write|Edit` into the memory folder can wake the sync too. That brings
  the file memory down to seconds. It is optional, and its cost is one hook entry.
- **Hub rolled back to 0.18.1 while an Orchestrator box runs.** The old hub can't read
  `BoxWork::Orchestrator`, treats the row as `Remains` and deletes the box without a flush. Up to
  20 s of file memory can be lost. Pinned memories are already on the hub. This is accepted, and
  the release note says so.

## 5. One Orchestrator at a time (requirement 5)

**Decision.** The account has one seat. While an Orchestrator box row exists and is not Ending,
the box holds the seat, whatever the machines' `host` settings say. Otherwise the seat goes to
the machine whose settings say `host`, as today. The `host` setting stays the user's choice of
machine. A box only borrows the seat.

- `relay::host` and the host check in `overview` call one new `seat(hub, account)`. It reads the
  Orchestrator box row first, then falls back to `the_host`. The box node never writes
  `host: true`, so `the_host` never sees two hosts.
- **Moving to a box by hand while the host is online.** If the host's Orchestrator is mid-turn,
  the start is refused (3.2 step 7). Otherwise the hub sends the host `FlushOrchestrator`, then
  `StopTurn`/`CloseTab` for its global tab, then writes the row. The box resumes from what was
  just pushed.
- **The host comes back online while the box runs.** The box keeps the seat until it goes idle,
  so a turn is never cut. The UI shows "Orchestrator is on a box · Move back to MacBook".
  **Move back** sends `EndOrchestratorBox`: flush, then delete. On the machine, the stale-tab
  rule (4.4) restarts its tab on the box's conversation.
- **An old app (0.18.1) holds the host setting.** The seat still goes to the box. The old app's
  own global tab gets "does not host" refusals from the hub, as a non-host does today. The old
  machine does not sync the share. So the UI says "<machine> needs pinned 0.18.2 to hand the
  Orchestrator's memory to a box" whenever the newest share is older than the host's last online
  time.

**Why.**

- Two live Orchestrators would split the conversation.
- They would also both write the file memory, whose sync keeps only the newer copy.
- Captain reports need exactly one destination.
- Keeping the machine as the default restores the user's setting with no extra step.
- Letting the box finish its idle period avoids losing a turn the user is waiting on.

## 6. Abilities

| Tool | Status on a box | Why, and what changes |
|---|---|---|
| `projects` | Works, with a small change | It runs on the hub's overview through the seat. The change leaves the Orchestrator's own box out of `machines`. Offline machines show their last-listed projects with no Captain, as today. |
| `sessions`, `read_session` | Works | They list and read the online machines, which includes Captain boxes. An offline machine's sessions can't be read. That is unchanged. |
| `waiting` | Works | It is hub overview data. |
| `send_to_captain` | Needs a change | Today, with no online checkout, it fails (`route_remote`). On a box (`PINNED_BOX_ROLE=orchestrator`), a project with no online checkout goes as `box: true` automatically, through `send_to_captain_box`. `machine` still forces a machine. The `orchestrator` (Captains) box permission, the cap, the per-account Claude token and the Captain Box budget apply, as today. A project with no git origin (`<machine>:<path>` keys) can't move to a box, and the tool says so. |
| `read_captain` | Works | It reads the Captain on any online machine or Captain box. |
| `tell_orchestrator` (from Captains), Captain reports | Works | `RelayTo::Orchestrator` delivers to `host()`, which becomes `seat()`. |
| `repo_overview`, `repo_read`, `repo_search` | Works when any online machine or Captain box has the checkout. Needs a change otherwise. | `relay::checkout` needs an online checkout. The fallback has the hub send the Orchestrator box `Call::Clone` with a `dest` under `~/.pinned/orquestrador/reads/`, which is a shallow, read-only clone that is not a project. Then it sends `Call::RepoLook` there. Rate limits, the repo insight switch and the audit stay. |
| `memory_search`, `memory_save`, `memory_forget`, prompt recall | Works after the sync change | It uses local SQLite on the box, filled and drained by 4.2. |
| Claude file memory (`MEMORY.md` through Read/Write/Edit) | Works after the sync change | The guard hook already allows the memory folder. |
| `actions`, `act` | Works | Hub actions run on the account. Machine actions run on the box node, or on a named online machine. An offline target fails, as today. |
| `delegation_stats` | Works | It is hub data. |
| Read, Write, Edit, Grep, Glob (guarded) | Works | The box has its own folder, scratchpad, memory folder and uploads. Captain scratchpads on boxes can be read. Those on offline machines can't. |
| Uploads (phone photos) | Works | `Call::Upload` goes to the seat machine. |
| `remind_me`, `list_reminders`, `cancel_reminder` | **Cannot work** | The desktop app keeps and fires reminders (`src/main.rs`). A box node has no app and no desktop, so the tools are not offered there. The box prompt says reminders are off. Reminders set on the MacBook stay there and fire when it is back. Follow-up idea: hub-side reminders typed into the Orchestrator chat when due (open question 6). |
| Anything on an offline computer | **Cannot work** | This covers files only on that disk, its uploads, its Captain's scratchpad, and `@machine` routing to it. The box only reaches what the hub and online nodes hold. |
| Repos that are not on GitHub | **Cannot work** on a box | Box clones use the account's GitHub token (`normalize_repo`, `github::hosts`). |

The prompt also changes. `global_orchestrator::prompt` gains a box variant, chosen by
`PINNED_BOX_ROLE`. It says the Orchestrator runs on a temporary box, that no computer may be
online, that `send_to_captain` opens Captains on boxes by itself, and that reminders are off.

## 7. Lifetime and idle delete

- The row runs under the same `reap_once` and `verdict` as Captain boxes, with one extra hold.
  The idle timer of an Orchestrator box does not run while any `BoxWork::Captain` row of the
  account is open (not Ending) or any Captain turn the Orchestrator started is in flight.
  Without this hold, a Captain box could report to an Orchestrator that was already deleted.
  `seat()` would then have no host, and the report would fail.
- Every other rule stays as it is: idle 10 min, exited 5 min, offline 10 min, max life 4 h, and
  the budget. The tab starts at boot (3.3), so the exited rule never fires on an Orchestrator
  that waits for its first message. (Open questions 2 and 3.)
- At the end, the flush runs (4.6), then the box and its machine go.
- The chat stays readable through `OrchestratorHistory`. A new start restores the memory (4.2,
  4.3) and continues the conversation (4.4).

## 8. Wire and HTTP changes (all additive)

New `HubCall` variants, from a signed-in client:

- `OpenOrchestratorBox { size: Option<BoxSize>, budget_usd: Option<f64> }` answers `BoxStatus`.
  It is idempotent while the row lives.
- `OrchestratorSeat` answers `OrchestratorSeat` (new struct):
  - `on`: `machine`, `box` or `none`
  - `machine: Option<MachineId>`
  - `box_status: Option<BoxStatus>`
  - `synced_at: Option<u64>`: when the share last changed on the hub
  - `can_box: Option<String>`: why a box can't start now, or none
- `EndOrchestratorBox` answers `null`. It runs the flush, then the delete.
- `OrchestratorHistory { before: Option<u64>, limit: Option<u32> }` answers `ThreadPage`.

New `Call` variant, from the hub to a node:

- `FlushOrchestrator` answers `{ pushed: u32, pulled: u32 }`. An older node refuses it as an
  unknown command, and the hub treats that as "nothing to flush".

Unchanged:

- `RelayTo`, `global_orchestrator::Settings`, `MachineView`, `BoxStatus` and the sync calls keep
  their shapes. The new sync key is only a new string value. No field goes onto a struct held
  inside an `Option`, which avoids the wire-checker trap with optional structs.
- No HTTP route is added or removed.

Not on the wire:

- `BoxWork::Orchestrator` is stored in `boxes.spec` (JSONB). It is not part of the wire.
- The hub adds the new paths to `sync::accepts`. A 0.18.1 hub refuses them, so the hub ships
  first. It deploys itself on merge to `main`.

Each Rust PR runs `cargo run -p pinned-server --bin wire-contract -- snapshot current` and
`check --base <base>`, which must show no new breaks.

## 9. Security

- **Upstash key.** It stays in the hub's account secrets, and only `Boxes` reads it. No new
  answer carries it. `OrchestratorSeat.can_box` is a fixed sentence, never an Upstash error. An
  error from Upstash is scrubbed as `usage()` does (`e.replace(&keys.api_key, "…")`). It never
  reaches the browser, the logs, the share or an agent.
- **Claude token.** It follows the per-account rule: `token_kept` runs before any row is
  written. It reaches the box only inside `box.env`, written through the Box API's file write
  with mode 0600, never on a command line, the same as Captain boxes. It is never written into
  the share.
- **GitHub token.** It goes into `box.env` as for other boxes, for the read-only clones in
  section 6. The Orchestrator has no Bash, so the agent can't use it directly.
- **Who can start a box.** A signed-in client, through the same gate as `open_box`. No agent
  tool opens an Orchestrator box. The Orchestrator itself can only open Captain boxes, under the
  existing `orchestrator` permission.
- **The seat.** Host-only calls are taken from the box's machine id only while its row lives.
  `seat()` checks the row and the machine id together, so a stale box node can't keep acting as
  host after its row ends.
- **What the box sees.** The box gets the Orchestrator share: memories, transcripts and the file
  memory. This is the account's own data, in a box opened with the account's own key. Captain
  boxes already pull their project's memory and transcripts this way. The hub's path filter keeps
  `.env`, keys and anything outside the listed paths out of the share.

## 10. Implementation in four PRs, in merge order

Rust builds and tests run on a **medium** box (a small box gets OOM-killed on
`cargo test -p pinned-core -p pinned-server`, pinned memory [101]). Never run them on the MacBook.

### PR 1. The Orchestrator share (Rust, medium box)

Files:

- `crates/core/src/sync.rs`: `ORCHESTRATOR_KEY`, the `Roots` `folder/` and `pinned/` roots, and
  `accepts`.
- `crates/core/src/memory.rs`: `uid`, `forgotten`, `export`, `merge`.
- `crates/core/src/machine.rs`: `orchestrator_share()`, the `pinned/settings.json` writer, the
  export after a save or forget, the waker, the stale-tab mark, and `Call::FlushOrchestrator`.
- `crates/core/src/link.rs`: the share in `Syncer::cycle`, and a cycle on demand for the flush.
- `crates/core/src/wire.rs`, `crates/server/src/hub.rs` (accepts), `wire-contract/current.json`.

Tests:

- memory.rs: merge is commutative. A forget travels. A free id is kept. A collision gets a new id.
- sync.rs, on the in-memory fake hub that two machines share: a memory saved on A appears in
  B's `memory_search`. `MEMORY.md` and a transcript travel. A path outside the list is refused.
- wire round trip for `FlushOrchestrator`.

### PR 2. The Orchestrator box lifecycle and the seat (Rust, medium box)

Files:

- `crates/server/src/boxes.rs`: `BoxWork::Orchestrator`, `BoxRole::Orchestrator`,
  `open_orchestrator`, `dispatch`, the idle hold, and the flush in `finish` with retries.
- `crates/server/src/relay.rs`: `seat()` used by `host`, `only_host` and `overview`, and the
  Orchestrator box left out of `projects`.
- `crates/server/src/hub.rs`: `OpenOrchestratorBox`, `OrchestratorSeat`, `EndOrchestratorBox`.
- `crates/server/src/box-start.sh`: trust the folder.
- `crates/server/src/main.rs`: `PINNED_BOX_POLICY_SECS`, which shortens idle and tick for the local proof only.
- `crates/core/src/machine.rs`: `PINNED_BOX_ROLE`, and resume of the newest synced transcript in
  `open_global_orchestrator`.
- `wire.rs`, `current.json`.

Tests in `crates/server/tests/hub.rs` with `FakeBoxes`, which already starts an in-process node.
They need the throwaway Postgres on 5499 (pinned memory "Hub tests Postgres"):

- No machine online, then open: the box node becomes the seat, and `projects` answers from it.
- `send_to_captain` opens a Captain box.
- The Orchestrator box outlives the Captain box, then goes idle.
- `FlushOrchestrator` arrives before `delete`, and the memory is in `project_files`.
- A second open answers the same row.
- With no token, it refuses with the ADD_TOKEN text and writes no row.
- A host machine coming online keeps the seat on the box until **Move back**.

One manual run on a real small box checks the cross-folder resume (4.4).

### PR 3. Abilities on a box (Rust, medium box)

Files:

- `crates/core/src/global_orchestrator.rs`: the box prompt.
- `crates/core/src/machine.rs`: the `send_to_captain` box fallback and the reminders note.
- `crates/server/src/relay.rs`: the repo read fallback through a read clone on the box.
- `crates/core/src/thread.rs`: `read_back` over bytes.
- `crates/server/src/hub.rs`: `OrchestratorHistory`.
- `wire.rs`, `current.json`.

Tests:

- On a box with no online checkout, `send_to_captain` answers `iniciado` with a box machine.
- `repo_search` with no online checkout reads the clone and writes an audit row.
- `OrchestratorHistory` pages the hub copy newest first, with the same items as `Call::Thread`
  on the same file.

### PR 4. UI and proof (no Rust build for its tests; the sweep needs a built `pinned-server`, so it runs on the medium box)

Files:

- `ui/global-chat.js`: `globalHostMachine` reads the seat.
- `ui/gui-sidebar.js`: `drawNav` and `openGlobalChat` states.
- `ui/gui-phone.js`.
- `ui/gui-global-orchestrator.js`: the Temporary box chip, Move to a box, and Move back.
- A new `ui/gui-orchestrator-box.js`: the start sheet, the boot progress and the read-only history.
- Stubs in the UI test harnesses for new cross-file globals (pinned memory "UI test harness stubs").
- `scripts/web-sweep/orchestrator-box.js`: the proof below.
- `docs/global-orchestrator/CONTRACTS.md` and `CHANGELOG.md`.

Tests:

- `node --test ui/global-orchestrator.test.js ui/global-chat.test.js`, plus new cases. The row
  stays live with no machine online, the sheet offers the box, and the seat on a box routes the
  chat to the box machine.

## 11. Proof plan for the tutorial video (local hub, fake data)

It runs on a local hub with the web-sweep stack (`scripts/web-sweep/stack.sh`).
`UPSTASH_BOX_BASE_URL` points at a stub Box API in the sweep script:

- `create`, `list`, `write` and `delete` keep a folder per box.
- `exec` of the bootstrap starts the locally built `pinned-server node --config-dir <box>/.pinned`
  with `HOME=<box>/home`, and `PATH` set to `scripts/web-sweep/fake-claude` and `fake-gh`. That
  is the same idea as `FakeBoxes` in hub.rs.

`fake-claude` runs scripted turns from `fake-turns.json`, whose `calls` make real pinned MCP tool
calls. The memory, projects and Captain steps therefore go through the real code. Recording
follows the `pinned:tutorial-video` skill: visible clicks only, never URL jumps.

1. **Seed.** A sweep node "MacBook" hosts the Orchestrator.
   - It saves the pinned memory "Deploys: deploys go through the VPS" and a `MEMORY.md` line.
   - Wait for `OrchestratorSeat.synced_at`.
   - Stop the node. The page shows 0 machines online.
2. **Start from a phone.** Open the page at 390x844 and sign in. Tap the Orchestrator tab. The
   sheet says no computer is online and shows the last conversation. Tap **Start on a temporary
   box**. Record creating, booting and running.
3. **Recall.** Type "where do deploys go?" The scripted turn calls `memory_search`, and the reply
   quotes "[1] Deploys: deploys go through the VPS". Open the Memory panel to show the same row.
4. **Projects.** Type "what projects do I have?" The turn calls `projects()`, and the reply lists
   the seeded projects, with MacBook offline.
5. **Captain on a box.** Type "ask site's Captain to fix the footer". The turn calls
   `send_to_captain` with no `box`. The fallback answers `iniciado` with `Box · Captain site`.
   The stub opens a second box, and a scripted Captain report arrives in the Orchestrator chat.
6. **Save a memory in the box.** Type "remember that the footer uses the brand font". The turn
   calls `memory_save` (id 2).
7. **Idle delete.** The sweep hub runs a short `Policy` (idle 30 s, tick 2 s). Today `main.rs` always uses `Policy::default()`, so PR 2 adds `PINNED_BOX_POLICY_SECS=idle,tick`, read once at hub start, which the stack script sets. The Captain box goes
   first, then the Orchestrator box. The stub logs `exec FlushOrchestrator` before `delete`, and
   the sidebar goes back to "No computer online".
8. **Memory survived.** Tap Orchestrator again. The sheet's history shows step 6's turn. Start a
   box again, type "what font does the footer use?", and the reply quotes "[2] …brand font".
   Then start the "MacBook" node again. Its Memory panel lists memories 1 and 2, which shows the
   way back to a machine as well.

The script asserts each step (`check(ok, …)`) and saves the screens. The video is recorded
before `pinned evidence`.

## 12. Open questions for Felipe

1. **Which permission gates the Orchestrator box?** This design uses the **Sessions** box
   permission, because the user's tap is the yes, as with Open box. The alternative is the
   **Captains** permission, since a box Orchestrator mostly opens Captain boxes.
2. **Idle time.** Captain boxes go after 10 min idle, held while any Captain box lives. You may
   think for longer than that between phone messages, and each new start costs a boot of a few
   minutes. Keep 10 min, or give the Orchestrator box 30 min?
3. **Max life of 4 h.** Every box ends at 4 h, whatever it does. Keep that for the Orchestrator,
   or let only the Box budget end it?
4. **Machine back online.** This design keeps the box until it is idle, with a **Move back**
   button. Do you want an immediate move back instead? A true memory conflict only happens if a
   machine's Orchestrator saved a memory while cut off from the hub. Is a new id in that case
   acceptable?
5. **User-level `~/.claude/CLAUDE.md`.** It is not the Orchestrator's own and is not carried.
   Should the box get it?
6. **Reminders.** They are off on a box. Is that enough for 0.18.2, or should hub-side reminders
   (typed into the Orchestrator chat when due) be planned as the follow-up?
7. **Which account the box signs in as.** It uses the account the host's Orchestrator ran under,
   from the share, else the default account. Is that the right default?
