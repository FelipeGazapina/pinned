# question_user — a questionnaire tool for every agent (0.18.2 design)

Status: draft design, no implementation. Base: main @ 5af27835 (v0.18.1).

## 1. Where it lands today

### 1.1 The Orchestrator asks after the fact

`crates/core/src/global_orchestrator.rs:777` `prompt()` is the global Orchestrator's system prompt
(Portuguese). It has exactly one rule about asking before sending, rule 3: "Se ainda houver dúvida,
pergunte ao Felipe antes de enviar qualquer coisa", and it only covers which machine. Nothing tells
it to ask about the *work* before `send_to_captain`, so it sends, and when something was unclear it
asks Felipe in its reply and later steers the Captain with a second message. That is the behaviour
to replace.

The only structured "ask" the Orchestrator has is the last-line convention at
`global_orchestrator.rs:799`:

```
Quando uma resposta termina com uma pergunta fechada ao Felipe, a última linha traz só as opções…
Opções: Sim | Ainda não
```

The app renders it in `ui/global-chat.js:35` `quickReplyOf()` (last non-empty line matching
`^(?:Opções|Options): (.+)$`), and `ui/gui-timeline.js:1092` draws it as `.quick-replies` buttons
(`ui/gui-orchestrator.css:71`) under the latest answer of the global lane only. One click types the
option as the next user message. Limits: one question, no recommendation, no "why", global lane only,
the turn is already over (so it is an *offer*, not a gate), and nothing in code knows a question is open.

`offer_of()` / `question_in()` (`global_orchestrator.rs:510`, `:519`) read a finished turn's last
question mark into `TabView.question`, which Needs you shows as an "Asked" card (`offer`, rank 4,
blocks nobody). Again: after the fact.

### 1.2 Existing ask, approval and plan flows

| Flow | Where it lives | How the user answers | Blocks the agent? |
|---|---|---|---|
| Claude Code `AskUserQuestion` / permission prompt | Claude Code's own TUI menu in the PTY. Node marks the tab `status: needs-input`, `running && interrupted` (`waiting_kind`, `global_orchestrator.rs:492`), `ask_detail` from the transcript (`thread.rs:793`) | `ui/redesign-model.js:88` `pendingAsk()` parses the menu **off the terminal screen** (`terminalMenu`), `answerAsk()` (`:304`) presses the option's key. Needs you `question` card (`ui/gui-needs-you.js:105`), worker banner (`ui/gui-worker-banner.js`) | Yes, inside Claude Code |
| `ExitPlanMode` plan | same PTY menu; `plan-card` in `ui/gui-timeline.js:770` | `answerAsk(..., choices[0])` | Yes |
| Agent `act` with a `confirm` action | Hub DB `agent_confirmations` (`crates/server/src/db.rs:105`), `surface::act` (`crates/server/src/surface.rs:90`) | Needs you `agent` card → `HubCall::DecideConfirmation` | **No**: `act` returns `pending` with `PENDING_NEXT` (`crates/core/src/surface.rs:611`); the decision comes back later as a typed `Call::Notice` (`surface.rs:367 notify`, `deliver_notices` for an offline machine) |
| Orchestrator "Opções:" | last line of a reply | quick-reply button types a message | No (turn over) |

What this means for `question_user`:

* The screen-parsed `AskUserQuestion` path is the closest UX, and it is fragile by design:
  `pendingAsk` gives `choices: null` ("only opening the thread can answer") for several questions in
  a row or multi-select, nothing works for headless Cursor/Codex children, and the hub web/phone can
  only answer what the screen parser saw. Claude Code's own `AskUserQuestion` also has no "skip",
  no "recommended" mark and no gate the node can see. A pinned-owned tool gives structured data that
  every surface renders the same way, for every provider.
* The confirmation flow shows the durable half we want: the request is stored, survives the asking
  machine going offline, and its answer reaches the session as a `Call::Notice` once it can.

### 1.3 How a tool call can block until the user answers

Pinned's MCP server for sessions is `pinned mcp` on stdio, `crates/core/src/reminders.rs:166`
`serve_io()`. Each `tools/call` runs on its own thread, so one call may wait while others go on.
`delegate_call()` (`reminders.rs:764`) is the existing blocking pattern: it sends one
`Message::Delegate` over the node socket (`send()`, `:872`), then polls `Message::TaskStatus` every
≤200 ms and writes `notifications/progress` every 10 s (`progress_every`) until the child ends or
`timeout_ms` passes; on timeout it returns a `next` that says a notice will be typed later.

`question_user` uses the same two halves: **block with progress while the session lives**, and
**fall back to a typed notice** (the confirmation path) when the wait ends without an answer
(Claude Code's MCP timeout, a pinned restart, the session resumed in a new tab). Blocking matters
for three agent kinds:

* a **worker's** task ends with its first finished turn (`delegate_task` description,
  `reminders.rs:295`); a non-blocking tool would make "ask, end turn" finish the task;
* a **scheduled/workflow run** is also a task ended by its first turn (`Call::RunNow`, `wire.rs:270`);
* the **Orchestrator** must stay in the turn that took the user's request, so that after the
  answers it writes the one complete message to the Captain.

### 1.4 Where tools are served

| Server | File | Who calls it | `question_user`? |
|---|---|---|---|
| Node MCP (`pinned mcp`, stdio, per session) | `crates/core/src/reminders.rs` (`tool_definitions` `:254`, `project_tools` `:436`, `call` `:713`); handled on the node in `crates/core/src/machine.rs:3283` `project_tool()` | every pinned session: Orchestrator, Captain, loose thread, worker thread, headless child (`PINNED_TASK`), workflow run, box sessions (`crates/server/src/main.rs:179` writes the same config on a box) | **Yes, here.** |
| Hub MCP for external agents (Streamable HTTP, OAuth) | `crates/server/src/mcp.rs` | claude.ai / other external clients through `/mcp` | **No** in 0.18.2: an external agent already has its own user in front of it; its header says "Nothing here answers a permission prompt". Revisit if an external agent ever runs unattended. |

## 2. The tool: schema and result
_(in progress)_

## 3. How each agent kind gets it
_(in progress)_

## 4. Orchestrator gate, enforced in code
_(in progress)_

## 5. No user there: scheduled and workflow runs
_(in progress)_

## 6. UI (desktop and hub web, phone width)
_(in progress)_

## 7. Wire changes (additive)
_(in progress)_

## 8. PR split, proof, model tier
_(in progress)_
