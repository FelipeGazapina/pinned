# Provider delegation

A pinned session can start a child agent on another account that is signed in on this machine. The child uses that account's subscription. Claude, Codex, and Cursor each have one driver: it builds the headless command and reads one stream line.

## Surface

The `pinned` MCP server exposes `orchestrator_capabilities`, `delegate_task`, `task_status`, and `task_cancel`. Arguments use the T3 names (`task`, `target.providerInstanceId`, `options.effort` or `reasoning_effort`, `isolation`, `mode`, `timeoutMs`, `interactionMode`). The server parses them once. The socket message is a typed `Request`.

`mode: "wait"` polls with short socket calls until the child finishes or `timeoutMs` (default 10 minutes). A caller with no tab always waits, because nothing can type a notice into it. An async child types one notice into the parent tab when that tab is not Working. Several finished tasks share one notice. Reading `task_status` of a finished task marks it delivered.

A node keeps the MCP server and passes `--no-reminders`, so delegation works and desktop reminders stay off. A Codex tab gets the same server through `-c mcp_servers.pinned...`.

## Access

The child is never allowed more than its parent. A bypassed parent gets full access, unless the request asks for plan, which is read-only. Cursor's automatic mode passes `--trust` and does not pass `--force`. `--trust` is workspace trust, not a permission bypass.

## What differs from the brief

- `Task.cwd` is the worktree path when isolation is `worktree`. There is no second path field. The worktree is removed only when the process fails to start, so "Open as thread" can resume there.
- A call with no tab and no parent task is stored as parent task `detached`. No card and no notice, because there is no session to wake.
- Codex advertises no model list. Any model id is accepted as `target.model`. The Codex fixture follows the documented JSONL schema and was not run live.
- A delegated card is a `Subagent` whose id starts with `task-`. The wire type gained no new field. Failed and cancelled cards use the existing stopped state.
- `Message::Delegate` boxes the request so the socket message stays small.

## Verification

- Unit tests cover argv per provider and access, the three stream fixtures, T3 argument names, Cursor model and effort mapping, and the capabilities shape.
- `crates/core/tests/delegate_mcp.rs` runs `pinned mcp` against a Machine and a fake `cursor-agent`. Status returns the fixture text `DONE`. Cancel kills the sleeper.
- Live, through `pinned-server mcp` and a real Machine socket: Cursor `composer-2.5` returned `PINEAPPLE` (session `ac3a5579-2254-45b1-b9b9-e58eef6d4511`). Claude `claude-haiku-5-5` returned `PINEAPPLE` (session `5759a3e2-c98b-4e28-9559-ffc5eecacde1`).
- `wire-contract check --base e1a84e1` reports no new breaks. A plain `check` still exits 1 for the Connector.machine breaks that already shipped.
- Hub tests need Postgres on `127.0.0.1:5499`. It was not running, so those tests did not exercise this change.
