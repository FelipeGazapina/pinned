# Worker boxes reaped while the parent machine was offline (0.18.2)

Incident: 2026-11-12/13 UTC. Four `delegate_task` workers on hub-opened Upstash boxes finished while the
delegating Captain's MacBook (machine 9272f4d8) slept. All four results were lost. When the MacBook woke,
`task_status` said "Box · … is gone from this account".

Read on `origin/main` @ fe944379. I read the code only. I did not reproduce anything.

## Cause in three sentences

A worker box sends its end report (summary, error, usage) through the hub. The hub only relays it live to
the parent machine and stores it nowhere. With the parent offline, the hub refuses it, so the box resends
it every 3 s from its own disk (`remote.rs:146-165`, `machine.rs:4914-4940`).

The box reaper does not know a report is undelivered. It saw the finished worker sit quiet and deleted the
box after 10 min (Idle) or 5 min (Exited). That deletion took the box's disk, which held the only copy of
the report, and the "box ended" notice to the parent also failed and was dropped (`boxes.rs:382-407`, `1357-1373`).

On reconnect, the parent asked the hub about a machine row that had been deleted. "this account has no
machine …" maps to "gone from this account", because nothing anywhere could answer with the result
(`machine.rs:4843-4861`, `hub.rs:446-453`).

The hub did not kill the workers because the laptop went offline. The work had ended. Only the report was lost.

## Answers

### 1. The "delegate call" every 3 s

The repeated call is `HubCall::RemoteEnded`, the worker's end-of-task report to its parent.

- Box side: `send_remote_ends` (`crates/core/src/machine.rs:4914-4940`) sends `HubCall::RemoteEnded { to, tab, task, end }`
  for each task that ended here for another machine. `end` is `RemoteEnd { state, summary, error, usage }`
  (`crates/core/src/delegate/mod.rs:255-268`). A failed send is retried at most every `REMOTE_LOOK` = 3 s
  (`machine.rs:76`, filter at `4922`).
- The hub logs the failure as "machine X's delegate call failed" for `Delegate | RemoteTask | RemoteEnded | RemoteAsked | Place`
  (`crates/server/src/http.rs:736-743`).
- Hub side: `remote::ended` (`crates/server/src/remote.rs:146-165`) calls the parent with `Call::RemoteEnded`.
  The parent was offline, so `send_call` returns None (`hub.rs:1112-1114`). `answer` then makes it
  `Refused("<name> is offline")` (`hub.rs:446-453`), and `ended` returns it as an `Err` because `!account.is_online(to)` (`remote.rs:154`).
  The box is not ended (`end_delegate` runs only after a delivery, `remote.rs:159-161`).
- Persistence: the box keeps the end "undelivered on disk until the hub passed it on"
  (`machine.rs:4911-4913`, `delegate/registry.rs:699-716`). The box retries for as long as the box lives.
  The hub has no table, queue or buffer for it. Once the box is deleted the end is gone, so in practice
  it was dropped.
- One related thing did persist. Before sending ends, the box pushes its task records (`record_now`, `machine.rs:4925-4927`)
  into the hub's `delegation_tasks` table (`RecordTasks`, `http.rs:769-775`). A `TaskRecord` (`delegate/metrics.rs:151-181`)
  holds usage, outcome and reason, but no summary. This is where the `box_usd` figures came from.

### 2. Where task state and result live

- The task and its state live in the parent machine's delegate registry, persisted on the parent's disk
  (`delegate/registry.rs:1346`). The task stays non-terminal with `remote.machine = <box machine id>` until a
  `Call::RemoteEnded` settles it (`machine.rs:4734-4741` → `registry.rs:658-668`). A status poll can also answer it
  (`check_remotes`, `machine.rs:4843-4861`, every `REMOTE_CHECK` = 60 s, `machine.rs:82`).
- The result (`summary`) exists in two places only: the box's registry on the box disk, and in flight.
  The hub keeps none of it. It keeps the box row (`boxes` table) and the metrics record (`delegation_tasks`), and
  the reaper deletes the box row.
- With the parent offline, nothing keeps the result for reconnect.

### 3. The reaper: which rule fired and what it ignores

- `reap_once` (`crates/server/src/boxes.rs:1489-1593`) judges a `Running` delegate row by
  `account.seen_any(<box machine>)` (`boxes.rs:1547-1551`, `hub.rs:1101-1108`). That reads **the box's own
  node and tabs**, not the parent's. `verdict` (`boxes.rs:382-407`) with `Policy::default` (`boxes.rs:92-108`) gives
  Idle when the box is quiet (no Working tab) for 10 min, and Exited when no tab runs for 5 min. Offline (10 min) applies to the
  **box's** node.
- Fired: a158b1307b550645 and 9e87285a70ce361a got Idle (10 min). ef358ad1c9b00886 and 262ea5070271996c got Exited (5 min).
  For box node 82d0343b, the `RemoteEnded` retries start at 23:52:58 and Idle fires at 00:02:56. That is exactly 10 min of quiet
  after its worker ended. These boxes died **after their work ended**, not mid-work. That matches pushed branches with green CI.
- Neither `verdict` nor `reap_once` considers an undelivered end report, the parent's presence, or
  `BoxWork::Delegate.origin`. The only use of `origin` at the end is `tell_origin` (`boxes.rs:1357-1373`). It tries
  once, with `ORIGIN_WAIT` = 30 s, to tell the parent "the box ended before its task did". The parent was offline, so it logged
  "could not tell … that box … ended" and dropped the notice. This is the 00:02:56 / 00:05:57 / 00:10:58 / 00:18:00 lines.
- **Ruled out:** the hub does not delete worker boxes because the owning machine went offline, because the Captain's node
  disconnected, or because the Captain session errored. No code path keys on those. The parent's absence mattered only because it
  blocked delivery, and the reaper's normal idle/exit rules then deleted the box that held the report.
  "machine 82d0343b disconnected" is the result of the reap, not its cause. `end` removes the machine and drops its socket
  (`boxes.rs:1337-1339`, logged at `http.rs:658`).
- With the parent online the same box ends `Done`: a delivered end calls `end_delegate` → `End::Done` (`remote.rs:159-161`, `boxes.rs:968-971`).
  That matches the earlier `Ending(Done)` boxes.

### 4. "is gone from this account" on reconnect

- On reconnect, `check_remotes` (`machine.rs:4843-4861`) sends `HubCall::RemoteTask { machine: <box machine> }`.
  The hub's `remote_task` (`remote.rs:120-144`) finds no box row (`delegate_box` → None, `boxes.rs:962-965`, row deleted at
  `boxes.rs:1352`). It calls the machine, which is offline with no DB row (`hub.db.remove_machine`, `boxes.rs:1337`). `answer` then says
  "this account has no machine …" (`hub.rs:450-452`). `machine_gone` matches that (`delegate/mod.rs:33-35`), and the parent fails the
  task with "… is gone from this account (a box that ended, or a machine someone removed), so its worker can't finish" (`machine.rs:4852`).
- It could not have fetched the result: no copy existed anywhere reachable. The only hub-side trace is the `delegation_tasks` record
  (outcome and usage, no summary). The parent could at most have said "it finished, but its report was lost" instead of implying it never finished.

### 5. Billing

- These were real deletes, not pauses. `end` calls `api.delete` → `upstash::delete` = HTTP `DELETE /v2/box/{id}` (`boxes.rs:1343-1351`,
  `crates/core/src/upstash.rs:378-384`). The log line "deleted box … (Ending(…))" comes after it (`boxes.rs:1354`).
- The boxes stayed Running from 23:40 until deletion. Nothing pauses a box (no pause path in `boxes.rs`). pinned prices a
  box at `BOX_USD_PER_HOUR` = 0.10 (`wire.rs:9`), whatever it does. At that rate, 23:40:58 → deletion is:
  a158… 22 min ≈ $0.037, ef35… 25 min ≈ $0.042, 9e87… 30 min ≈ $0.050, 262e… 37 min ≈ $0.062 (≈ $0.19 total). Not all of
  that is waste. Each worker was still working until its end, roughly 5–10 min before its deletion by the rules above. The cost of the
  lost delivery is only the idle tail, ≈ $0.008–0.017 per box. Whether Upstash bills an idle running box at the full rate is outside
  this repo. These figures use pinned's own price.
- The per-task `box_usd` of about $0.013–0.017 is the box time the **task records** billed (`box_time` → `billed_box_ms`,
  `boxes.rs:1377-1387`, `db.rs:1140-1151`). That is about 8–10 min of task time, not the box's whole life. `with_box_time`: `boxes.rs:1389-1395`.

### 6. The 23:40:08 gh scope errors from "client 3"

- "client {client}'s call to {machine} failed" is the app (a client socket, here the MacBook's own app) calling a machine
  through the hub (`http.rs:557-568`). The error text is gh refusing a token that has only the `repo` scope. The client call to every
  machine that runs gh with the account's token is the GitHub list, `Call::PullRequests` → `pulls::list` (`machine.rs:1277-1280`,
  `pulls.rs:306/489/531`). Its gh search needs more than `repo` (likely `read:org`). This is inferred from the message, because the log line
  does not name the call.
- It is unrelated. It is a read-only UI fetch that failed on the boxes 50 s before the laptop slept. It changes no task or box
  state, and the reaper never reads it. It is a separate small bug: the boxes' GitHub token scope, or the PR list needs to degrade gracefully.

### 7. Same path elsewhere

- **A Captain on a box, delegating workers to other boxes.** The workers' `RemoteEnded` goes `to` the Captain box. The Captain box is
  judged by `seen_any` of its own tabs (`boxes.rs:1547-1551`). A Captain that has delegated and ended its turn is quiet, so its box can be
  Idle-reaped after 10 min **while its workers still run**. Nothing holds a box while delegate rows name it as `origin.machine`. Its
  workers then hit "this account has no machine" → `Refused` + offline → `Err` (`remote.rs:153-154`). They retry until their own boxes
  are reaped, the same loss. This is by reading, not reproduced.
- **A Captain's report to an offline Orchestrator host.** `report_up` → `HubCall::Relay { RelayTo::Orchestrator }` → `relay::host`
  needs an online host (`relay.rs:59-72`, `395-420`). It fails with "no online machine hosts the global orchestrator". The Captain logs
  it once and drops it (`machine.rs:3274-3279`), with no retry and no persistence. That is worse than the worker path, which at least retries.
- **Planned Orchestrator-on-a-box** (`feat/orchestrator-box`, `.claude/research/orchestrator-box/DESIGN.md` §7). It holds the
  Orchestrator box's idle timer while any `BoxWork::Captain` row exists, so a box Orchestrator outlives its Captains. It does not cover:
  (a) workers whose parent Captain box was reaped (above), (b) Captain reports while the seat is briefly unhosted (handover, restart),
  or (c) a laptop host offline in the non-box setup. All of them need the same thing: the hub keeps an undeliverable
  report and hands it over when the recipient appears.

## Recommended fix

Do **A**, plus the small reaper hold **B′**. Do not hold boxes until the parent returns (B), because that pays for idle boxes
for a whole night of sleep and still loses the report at the cap.

**A. The hub keeps undelivered worker ends (main fix).**

1. New hub table `held_ends(account, origin_machine, origin_tab, task, from_machine, end jsonb, held_at)`, primary key
   `(account, origin_machine, task)`. This is server DB only and not wire.
2. `remote::ended` (`remote.rs:146`): when the parent is offline (`Refused` + `!is_online(to)`), insert the end and return `Ok(true)`
   instead of `Err`. Then call `end_delegate` as for a delivered end. The box marks it sent, stops retrying, and is deleted as `Done`
   within seconds instead of 5–10 min idle, which saves the tail cost.
3. `Boxes::tell_origin` (`boxes.rs:1357`): on failure, insert its failed `RemoteEnd` (do not overwrite a held real end).
4. Delivery: when the origin machine comes online (`Account::came_online` / `http.rs:624`), send each held end as the
   existing `Call::RemoteEnded { from, tab, task, end }` and delete the row once the answer is `Ok`. `settle_remote` already
   accepts `from` = the box's machine id and a renamed tab (`registry.rs:658-668`).
5. Read path: in `remote::remote_task` (`remote.rs:120`), before calling a machine with no box row, look for a held end and
   answer `{ workState: <end.state>, summary, error, usage }`. `settle_from` (`machine.rs:4848`) then settles the task. This closes
   the race where `check_remotes` runs before the flush.
6. Cap: delete held ends older than 30 days, or when their origin machine is removed. Clamp `summary` to the size the registry
   already accepts.

**B′. Hold a box that others delegate from.** In `reap_once`, skip the Idle and Exited verdicts for a `Running` row while any
`BoxWork::Delegate` row has `origin.machine` = this row's machine, keeping max life and budget. This covers a Captain on a box
whose workers are still out. With A in place, a parent box's absence alone no longer loses a report. B′ only keeps the
parent box alive to read it.

**Captain → Orchestrator reports** (Q7) are the same idea for `RelayTo::Orchestrator`: hold the report on the hub when no host
answers, and deliver it to the next host. That is a follow-up PR, which `feat/orchestrator-box` should plan on.

### Wire changes

None needed. A, step 2 changes only what the hub replies to an existing call (`Ok(true)` instead of `Err`). Old boxes already treat
any `Ok` as delivered (`machine.rs:4931-4937`). Step 4 reuses `Call::RemoteEnded`. Step 5 answers `RemoteTask` in the shape
`check_remotes` already reads. If a marker is wanted, add an optional `held: bool` to the reply value, which is additive. `wire-contract check`
should stay at exit 0, and `current.json` should not change.

### Tests that reproduce it

In `crates/server/tests/hub.rs`, next to the existing "is gone" test around `hub.rs:5314`, with the fake box API:

1. `a_worker_end_reaches_a_parent_that_was_offline`: parent machine delegates to a box → parent disconnects → box node sends
   `RemoteEnded` (result_available, summary "done") → the hub answers Ok, and the box row ends `Done` with a fake `DELETE` → parent
   reconnects → parent receives `Call::RemoteEnded` with the summary. Today this fails: the hub answers `Err("… is offline")` and the box is never ended.
2. `a_reaped_box_end_is_kept_for_its_parent`: parent offline → `reap_once` with timers past `idle` → parent reconnects →
   `RemoteTask` answers `failed` with "the box ended before its task did", not "this account has no machine".
3. `remote_task_answers_a_held_end`: held row present, no box row → `RemoteTask` returns the held state and summary.
4. `a_box_others_delegate_from_is_not_idle_reaped` (B′).
5. Core: in `machine.rs` tests, `check_remotes` settles a task from a `result_available` answer for a box machine
   (this likely passes already, as a guard).

### Risk

Low to medium. The new behaviour is that the hub reports "taken" for an end the parent has not seen yet. If the held row's
insert fails, the hub must keep returning `Err` (insert first, then `Ok`), or the end is lost silently. Double delivery is
harmless: `settle_remote` ignores terminal tasks (`registry.rs:665`). A flush on reconnect must not block `came_online`, so spawn
it. Ends for a parent tab closed in the meantime go to `settle_remote`, which returns false, and are then dropped.

### Model tier for the implementation PR

Opus (high tier). The work spans a DB migration, the hub's call and reconnect paths, the reaper, and ordering/race
reasoning, with a wire-contract check to keep green. A Sonnet-tier model could do test item 5 and the gh-scope follow-up separately.
