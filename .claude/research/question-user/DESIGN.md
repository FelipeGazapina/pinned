# question_user — a questionnaire tool for every agent (0.18.2 design)

Status: draft design, no implementation. Base: main @ 5af27835 (v0.18.1).

**In one paragraph.** A pinned MCP tool, `question_user`, offered to every session pinned starts
(Orchestrator, Captain, loose thread, worker, headless child, box session, workflow run). The agent
passes 1–4 questions, each with 2–4 options, its recommendation first and marked; the call
**blocks** until the user answers, accepts the recommendations with one click, types an answer, or
skips (with a warning). The questionnaire is stored on the node, travels on `TabView.questions`, and
turns the tab into a mid-turn `needs-input`, so `waiting()`, Needs you, the sidebar, notifications
and the hub web light up with no new plumbing; a new `Call::AnswerQuestions` carries the answer back.
For the global Orchestrator, `send_to_captain` to a project its open questionnaire names is refused
in code, so the Captain gets one complete message. Unattended runs wait; they never skip by
themselves. All wire changes are additive.

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

One tool, `question_user`, defined once in `reminders.rs` as an `Audience::Everyone` entry of
`project_tools()` (so it is also in `permissions()` and pre-allowed as `mcp__pinned__question_user`).
The description carries the rules, because it is the one text every agent kind sees, including
Codex/Cursor children that never read pinned's system prompts.

### 2.1 Input

```jsonc
{
  "name": "question_user",
  "description": "Ask the user before you act, when an answer would change what you do. Each question has 2-4 options, your recommendation first with recommended: true; the user can also type an answer, or skip. Ask only questions whose answer changes what happens next, never ones you could answer by reading the code, the task or your memory, and never invented ones: with no open question, do not call this. Ask everything you need in one call. The call waits until the user answers or skips. answered: go on with the answers. skipped: go on without them, take your recommended option for each, and state each assumption where your work is read (a worker in its final reply, the global orchestrator in its message to the Capitão and in its reply to the user). pending: the wait ended first; end your turn, the answers arrive as a message that starts with [pinned] Answers.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "title":   { "type": "string", "description": "What the questions are about, in a few words, e.g. \"Checkout redesign\"." },
      "questions": {
        "type": "array", "minItems": 1, "maxItems": 4,
        "items": {
          "type": "object",
          "properties": {
            "question": { "type": "string", "description": "One question, ending in ?" },
            "header":   { "type": "string", "maxLength": 16, "description": "A chip label, e.g. \"Branch\"." },
            "changes":  { "type": "string", "description": "What you do differently depending on the answer. Required: a question whose answer changes nothing is not asked." },
            "options": {
              "type": "array", "minItems": 2, "maxItems": 4,
              "items": {
                "type": "object",
                "properties": {
                  "label":       { "type": "string", "maxLength": 60 },
                  "description": { "type": "string", "description": "One line: what choosing it leads to." },
                  "recommended": { "type": "boolean" }
                },
                "required": ["label"]
              }
            },
            "multi_select": { "type": "boolean", "default": false }
          },
          "required": ["question", "changes", "options"]
        }
      },
      "projects": {
        "type": "array", "items": { "type": "string" },
        "description": "Global orchestrator only, and required there: the project keys (from projects) whose message waits on these answers. send_to_captain to them is refused until the user answers or skips."
      }
    },
    "required": ["questions"]
  }
}
```

Validation on the node (refused with a message the agent can fix, before anything is shown):

* every question has exactly one `recommended: true` option, and it is `options[0]`
  ("put your recommended option first and mark it recommended");
* 1–4 questions, 2–4 options each, labels unique within a question, text cut to sane lengths
  (question 300, description 160, changes 200 chars);
* `projects` only from the global orchestrator, and required there (each key must resolve as
  `send_to_captain`'s `project` does, via `local_project` / the hub overview);
* one open questionnaire per tab: a second call while one is open is refused with its id
  ("questions q-7 are still open: wait for them"). A parallel tool call cannot sneak in a second set.

### 2.2 Result (the tool's text, JSON)

```jsonc
// answered
{ "outcome": "answered", "id": "q-7",
  "answers": [
    { "header": "Branch", "question": "Base the fix on main or on release/0.18?",
      "chosen": ["main"], "recommended": true },              // accepted the recommendation
    { "header": "Scope",  "question": "Also fix the phone layout?",
      "chosen": [], "text": "Only the desktop one, the phone is being redone" } // typed
  ],
  "next": "Go on with these answers." }

// skipped
{ "outcome": "skipped", "id": "q-7",
  "next": "The user skipped these questions. Go on without them: take your recommended option for each and state every assumption you made where your work is read. Skipping can lead to wrong actions, so keep the assumptions easy to undo." }

// stopped: the user stopped the turn (Esc / Stop), which closes the questionnaire
{ "outcome": "cancelled", "id": "q-7", "next": "The user stopped this turn. Do not go on with the work these questions were about." }

// the blocking wait ended first (MCP timeout, pinned restarted, session moved tab)
{ "outcome": "pending", "id": "q-7", "next": "The questions are still open. End your turn now; the answers arrive as a message that starts with [pinned] Answers to q-7." }
```

The late notice (`pending` path) is a `Call::Notice`-style typed text, so it reads well to the model
and in the transcript:

```
[pinned] Answers to q-7 (Checkout redesign):
- Branch: main (your recommendation)
- Scope: "Only the desktop one, the phone is being redone"
```
or `[pinned] The user skipped q-7 (Checkout redesign). Go on with your recommended options and state your assumptions.`

### 2.3 Blocking, timeouts, restarts

* `reminders.rs` handles `question_user` like `delegate_call`: one `Message::Project { tool:
  ProjectTool::QuestionUser {..} }` opens it and returns the id, then it polls
  `ProjectTool::QuestionStatus { id }` every 500 ms and writes `notifications/progress`
  ("waiting on the user") every `progress_every()`, until the outcome is not `open`.
* Claude Code's MCP tool timeout: pinned starts every session, so it sets `MCP_TOOL_TIMEOUT` in the
  session env to a day (PR 1 checks the CLI's current default and how progress notifications reset
  it). The MCP thread stops waiting at 23 h and returns `pending`; the questionnaire stays open.
* The questionnaire lives on the **node** (`Machine.questions`, persisted to
  `<config>/questions.json` like the `captain-requests.json` ledger), keyed by tab and the tab's
  Claude `session`, so it survives a pinned restart. When the asking MCP call is gone and the user
  answers, the node delivers the notice to the tab (or the tab that resumed that `session`), queued
  until idle exactly like `write_notice`/`queue_request` (`machine.rs:5652`, `:2940`).
* `Call::StopTurn` on a tab closes its open questionnaire as `cancelled`.
* Archiving or closing the tab drops it (`cancelled`, no notice).

## 3. How each agent kind gets it, and where the questionnaire shows

Offered to every `Seat` in `tool_definitions` (`reminders.rs:254`): global orchestrator, Captain,
taste chat, plain thread, worker thread and delegated child (`child == true`), like the
`memory_*` tools. Project tools reach the node through `in_tab` (`reminders.rs:867`), which needs
`PINNED_TAB`; a headless child that has none sends `ProjectTool::QuestionUser` with its
`PINNED_TASK` instead, and the node resolves the tab that shows that task.

The questionnaire always belongs to **a tab**, and the node puts it on that tab's view (wire §7).
Every surface reads it from there:

| Agent | Tab the questionnaire is on | Where the user sees it |
|---|---|---|
| Global Orchestrator | its own tab | inline in the Orchestrator chat (the user is usually there); Needs you card; desktop notification |
| Captain | the Captain's tab | inline in the Captain thread; Needs you; the Orchestrator's `waiting()`; a strip in the Orchestrator chat (§6.4) |
| Loose thread | its tab | inline; Needs you; sidebar dot |
| Worker thread (Claude Code, `delegate_task` with a tab) | the worker's own tab | inline in the worker thread; the parent's **worker banner** over the composer (`gui-worker-banner.js`), which already steps through waiting workers; Needs you; `waiting()` |
| Headless child (Cursor/Codex, `PINNED_TASK`, no PTY) | the tab that shows the child's steps (`delegate/mod.rs:474`); if none, the parent tab, labelled "from worker <title>" | same as a worker thread |
| Session on an Upstash Box | its tab on the box machine (a box is a hub-linked machine) | hub web and the desktop app through the hub; Needs you, `waiting()` — nothing box-specific |
| Workflow run (Linear/Slack trigger, Run now, `Called`) | the run's tab (`Call::Run` / `RunNow`, has `connector`) | Needs you and `waiting()` first, since nobody has the tab open; see §5 |

### 3.1 Status: one switch makes every existing surface light up

While a tab has an open questionnaire the node reports it as a mid-turn question, the same state
Claude Code's own `AskUserQuestion` produces:

```
status = NeedsInput, running = true, interrupted = true,
message = "Claude has questions", ask_detail = <first question>
```

and adds the structured questionnaire beside it (`TabView.questions`, §7). So with **no new code**:
`waiting_kind()` returns `NeedsInput` and `ask_of()` returns `Question` (the Orchestrator's
`waiting()` lists it with its machine and project); the sidebar dot, `threadState() == "needs"`,
`navCounts().needsYou`, the desktop notification (`ui/main.js:278`) and an older app's Needs you
"Question → Open" card all work. The new UI only adds the one-click questionnaire on top.

The node must hold that status against Claude Code's hooks while the MCP call waits (hooks report
`Working`) and against `clear_answered_asks` (`machine.rs:5682`, which clears a menu ask once the
screen moves on). So the override is applied where the view is built ("open questionnaire ⇒
needs-input, interrupted"), never written into the tab's raw `status`; the few places that read raw
status for "waits on the user" (`note_user_waits`, §5) also ask the questions store. `waiting_kind()` keeps returning `None` for `orchestrator_global`; its own
questionnaire shows in its chat and in Needs you from `TabView.questions` directly.

### 3.2 A worker's question: directly to the user (recommended)

Options considered:

1. **Direct to the user** (recommended). The worker's questionnaire shows in its thread, in its
   Captain's worker banner, in Needs you and in the Orchestrator's `waiting()`. The user answers
   once, with one click, from wherever they are.
2. Through its Captain and the Orchestrator: the worker asks the Captain, which re-asks the
   Orchestrator, which re-asks the user. Two LLM hops that rewrite the options and lose
   "recommended", two extra turns, a Captain that may be busy (its messages queue, `queued`), and the
   one-click answer is gone. The Orchestrator gate would also have to reason about questions that are
   not its own.

Why direct is right: the question's owner is the one blocked, and pinned already surfaces a waiting
worker everywhere (`workersOf().ask`, worker banner, `waiting()`). What the Captain does get:

* the worker banner in its thread while the question is open (so a Captain that is the user's
  current view shows it);
* after the answer, the Q&A is in the worker's transcript and in the worker's final summary
  (the tool description makes the worker state answers/assumptions in its final reply);
* the Captain may answer **instead** of asking the user only by being asked: the worker tool
  description says "if the task text or your parent could answer it, it is not a question for the
  user". No routing to the parent in 0.18.2; `question_user { to: "parent" }` is a possible later
  addition.

The same applies to a Captain working on an Orchestrator request: its question goes straight to the
user (shown in the Orchestrator chat strip, §6.4). `captain_prompt()` (`global_orchestrator.rs:807`)
changes "use tell_orchestrator for a question it must answer" to: a question only the user can
answer → `question_user`; a question the Orchestrator can answer from the request, or news →
`tell_orchestrator`.

## 4. The Orchestrator gate, enforced in code

### 4.1 Prompt

`prompt()` in `global_orchestrator.rs` gets a new numbered rule between 2 and 3 (Portuguese, like the
rest), and rule 3's "pergunte ao Felipe antes de enviar" points at it:

> 2b. Antes de enviar a um Capitão, veja se falta algo que só o Felipe sabe e que muda o que o
> Capitão vai fazer (escopo, base, máquina, o que fica de fora). Se faltar, chame question_user
> **antes** de send_to_captain, com todas as perguntas de uma vez, a opção que você recomenda
> primeiro e marcada recommended, e em projects os projetos cuja mensagem espera a resposta. Não
> invente perguntas: sem dúvida real, envie direto. Primeiro envie com send_to_captain o que não
> depende de resposta (outros projetos da mesma mensagem não esperam); depois pergunte. Enquanto
> as perguntas estão abertas, send_to_captain para esses projetos é recusado: é o pinned que garante
> que o Capitão recebe uma mensagem completa, sem correção no meio do trabalho. Com as respostas,
> mande **uma** mensagem completa ao Capitão. Se o Felipe pular, siga com as opções recomendadas e
> escreva as suposições na mensagem ao Capitão ("Suposições: …") e na sua resposta ao Felipe.

The "Opções: A | B" last-line rule stays for what it is good at: an end-of-turn offer
("Quer que eu faça a release?") that blocks nobody. The prompt says so: "Opções só para uma oferta
no fim da resposta; para algo que decide o que enviar, use question_user."

### 4.2 Code

The open questionnaires live in a small module, `crates/core/src/questions.rs`:

```rust
pub struct Questions { open: Vec<Open>, path: Option<PathBuf> }   // persisted like global::Ledger
pub struct Open { id: String, tab: TabId, session: Option<String>, task: Option<String>,
                  asked: Questionnaire, projects: Vec<String /* resolved project keys */>,
                  at: u64, outcome: Option<Outcome>, notified: bool }
impl Questions {
    pub fn open(&mut self, tab, session, task, asked, projects) -> Result<String, String>;
    pub fn status(&self, id) -> Option<&Outcome>;
    pub fn answer(&mut self, tab, id, reply: Reply) -> Result<(), String>;   // Call::AnswerQuestions
    pub fn cancel_tab(&mut self, tab);                                       // StopTurn, close, archive
    /// The open questionnaire of `tab` that holds back `project`, if any.
    pub fn holding(&self, tab: &str, project: &str) -> Option<&Open>;
}
```

In `Machine::project_tool()` (`machine.rs:3283`) both `SendToCaptain` arms
(`machine.rs:3317` with `box`, `:3321` without) check the gate **after** `only_global()` and
**before** any routing (edition 2021, so a plain check inside the arm, not an `if let` guard):

```rust
ProjectTool::SendToCaptain { project, message, request_id, machine, boxed } => {
    only_global()?;
    self.questions_gate(tab, &project, machine.as_deref())?;   // new
    match boxed { Some(boxed) => …send_to_captain_box…, None => …send_to_captain… }
}

/// Refuses a send while this conversation's open questions hold `project` back.
fn questions_gate(&self, tab: &str, project: &str, machine: Option<&str>) -> Result<(), String> {
    let key = self.project_key(project, machine);           // same resolution projects() lists
    match self.questions.lock().unwrap().holding(tab, &key) {
        Some(open) => Err(format!("questions {} are open in this conversation about {}: wait for the answers or the skip, then send one complete message", open.id, open.name_of(&key))),
        None => Ok(()),
    }
}
```

The gate keys on the **resolved** project (its repo key, falling back to `<machine>:<path>`), so
`site`, `github.com/o/site` and `m-book:/w/site` all hit it. A questionnaire from the Orchestrator
without `projects` is refused at `question_user`, so the gate never blocks everything by accident,
and other projects in the same user message go out.

Since `question_user` blocks the Orchestrator's turn, the gate is mostly hit in three cases, which
is exactly what it is for: a `send_to_captain` issued in the same parallel tool batch as
`question_user`; a send after the wait returned `pending` and the model did not end its turn; a new
user message typed while the questionnaire is open (Claude Code feeds it to the model mid-turn).

Considered and left out: refusing `question_user` for a project that already has an in-flight
request (`Ledger::in_flight`). It would catch "send, then ask", but it also blocks a real new
request to a busy Captain. The prompt and the gate cover the case Felipe described.

### 4.3 Tests (PR 2), next to the existing ones in `machine.rs`

Modelled on `global_orchestrator_routes…` at `machine.rs:10600` (real socket, `recording_claude`):

1. `send_to_captain_waits_for_the_questions_that_hold_its_project` — global tab opens
   `question_user { projects: [site] }` → `send_to_captain site` is `Err("questions q-1 are open …")`
   under all three spellings of the project; `send_to_captain elsewhere` goes out (`Queued`/`iniciado`).
2. …after `Call::AnswerQuestions { reply: answered }` the same send goes out, and the Captain's
   recorded first prompt is the one message (no second `[Orchestrator]` prompt).
3. …after `reply: skipped`, same; the tool's result is `skipped` with the `next` text.
4. `Call::StopTurn` on the global tab cancels it and frees the gate.
5. A plain thread or Captain passing `projects` is refused; the Orchestrator omitting it is refused.
6. A second `question_user` while one is open is refused, with the open one's id.
7. The gate survives a reload of `questions.json` (new `Machine` on the same config dir).
8. `global_orchestrator.rs`: the prompt names `question_user` before `send_to_captain`, the gate
   and "Suposições"; the `captain_prompt` test (`:1193`) gets the new sentence.
9. `reminders.rs`: `question_user` is offered to every seat including a child (extend
   `a_session_gets_the_project_tools_a_delegated_child_lacks` `:1256` and the global list at `:1444`);
   an over-socket test like `a_wait_returns_the_summary…` (`:1058`) shows the call blocks, sends
   progress, and returns `answered` / `skipped` / `pending` (short `PINNED_QUESTION_WAIT_MS`).
10. `crates/server/tests/hub.rs`: an answer sent from a hub client to a tab on another machine
    reaches it (relay), and the Orchestrator's `waiting()` lists the Captain's open questionnaire.

## 5. No user there: workflow runs and other unattended sessions

Default: **the run waits on the user and shows in `waiting()`; it never skips by itself.**

* The run's tab gets the questionnaire like any tab (§3), so it is `needs-input`, in Needs you with
  its workflow's name as the card's `where` ("Workflow · Fix Linear bugs · Mac mini"), in the
  Orchestrator's `waiting()` (`ask: question`, `detail` = first question), with a desktop
  notification.
* It does not time out into a skip. The MCP call returns `pending` after 23 h; the agent ends its
  turn. For a task (a worker thread, `RunNow`), "its first finished turn ends the task"
  (`machine.rs:449`, `:1838`, `:5326`) becomes "its first finished turn **with no open
  questionnaire**": a turn that ends while one is open does not end the task, the answer's notice
  starts the next turn, and that turn's end does.
* Time limits: `Machine::note_user_waits` (`machine.rs:5564`) pauses the clock of every task
  whose tab waits on the user mid-turn (`status == NeedsInput && interrupted`), and
  `Delegates::note_user_waits` keeps it stopped (`delegate/mod.rs:2848`, "ten minutes on an
  approval are not work"). Its filter reads the tab's raw status, not the view, so PR 2 adds
  `|| questions.open_on(tab)` there, with a test. Budget is unaffected: a waiting session spends
  nothing.
* Boxes: the hub's box watcher ends a worker's or Captain's box after `idle` (10 min) of `quiet`,
  and `quiet` is "no tab is `Working`" (`crates/server/src/hub.rs:1096` `seen`, `:1106`
  `seen_any`; `boxes.rs:404`). A tab with an open questionnaire counts as not quiet there, so a box
  session that asks keeps its box until `max_life` or its budget ends it; then the questionnaire
  closes as `cancelled` and Needs you shows the box's end as it does today.
* A per-workflow setting "When it asks: wait for me | go on with its recommendation" is a later,
  separate change (it is a user choice that makes skipping automatic, so it must be explicit and
  visible on the workflow's page). Not in 0.18.2.
* Slack-triggered workflows could post the questionnaire into their Slack thread with buttons; out
  of scope for 0.18.2, noted because the questionnaire's shape (header, options, recommended) maps
  onto Block Kit one-to-one.

## 6. UI (desktop app and hub web; same `ui/` code)

The hub serves the same `ui/` folder (`crates/server/src/http.rs:137` `ServeDir`), and `callOn()`
(`ui/main.js:50`) reaches a tab's machine locally or through the hub, so there is one
implementation. A new `ui/gui-questions.js` (+ `gui-questions.css`) renders one questionnaire from
`tab.view.questions` and is used by three surfaces. Visual language: the `plan-card` shell
(`ui/gui-timeline.css:160`, 22 px radius, raised background, "Plan" badge → "Questions" badge) and
the Needs you option buttons (`.ny-option`, `.ny-label`, `.ny-about`, 48 px min height, 44 px tap
targets, `.ny-typed` free-text row) so it looks like what is already there.

### 6.1 Inline in the conversation (`gui-timeline.js`)

`shownHtml()` (`gui-timeline.js:782`) gets a branch for the `mcp__pinned__question_user` tool step:
while the tab's `view.questions.id` matches the step and it is open, draw the live card; once
answered or skipped, draw a compact `tool answered` row like `questionHtml()` (`:728`) does for
`AskUserQuestion` ("Questions · Branch: main · Scope: Only the desktop…" / "Questions · Skipped").
The live card also pins to the bottom of a running turn (`liveHtml`), so the questions are where
the eye is.

Desktop width (thread column ≤ 760 px):

```
╭──────────────────────────────────────────────────────────────╮
│ [Questions]  Checkout redesign                     1 of 2 ●○ │
│                                                              │
│ BRANCH                                                       │
│ Base the fix on main or on release/0.18?                     │
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ ◉ main                                    Recommended    │ │
│ │   Ships in 0.18.2 with the rest                          │ │
│ └──────────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ ○ release/0.18                                           │ │
│ │   Hotfix now, merge forward later                        │ │
│ └──────────────────────────────────────────────────────────┘ │
│   Type another answer…                                       │
│                                                              │
│ SCOPE                                                        │
│ Also fix the phone layout?                                   │
│ ┌ ◉ No, desktop only           Recommended ┐ ┌ ○ Yes ┐       │
│   Type another answer…                                       │
│                                                              │
│ ⓘ Changes: which branch the Capitão bases the PR on.  (why)  │
│                                                              │
│  Skip                            [ Accept recommended ]  [Send]│
╰──────────────────────────────────────────────────────────────╯
```

Behaviour:

* Each question starts **pre-selected on its recommendation** (`options[0]`, "Recommended" tag in
  `--g-warning-fg`'s neighbour token, like `.ny-kind`). So **Accept recommended** is one click and
  sends everything as recommended; changing any choice turns the primary button into **Send**.
  With one question, tapping an option sends at once (like the Needs you `option` button today).
* Free text: a `.ny-typed` row under each question; typing selects "typed" for that question.
  Dictation works because it is a normal input (`gui-dictation.js` binds to inputs).
* `multi_select` questions use checkboxes, recommended pre-checked.
* The "changes" line is behind a small ⓘ per question (collapsed by default; it is for the user
  who wants to know why).
* **Skip** is a quiet text button (`.ny-quiet`). It asks once, inline, not a modal:

```
│  Skip these questions?                                       │
│  The agent goes on with its own assumptions (its             │
│  recommendations), which can lead to wrong actions.          │
│                                   [ Back ]  [ Skip anyway ]  │
```

* While sending: the fieldset is disabled ("Sending…", `NEEDS_YOU_NOTES`); after: the card
  collapses to the answered row. If the questionnaire changed or closed meanwhile, the same
  "missed" note as Needs you.
* The composer stays usable. Its placeholder becomes "Answer the questions above, or type to add
  to them"; a typed message goes to the session as today (Claude Code queues it until the tool
  returns), and the gate still holds for the Orchestrator.

### 6.2 Phone width (hub web and the desktop app's narrow layout, ≤ 400 px)

Options stack full width, the action row sticks to the card bottom, the header compresses, the
"1 of N" dots step through questions one at a time (one question per screen) so nothing scrolls
sideways:

```
┌ 390px ───────────────────────────┐
│ ‹ Orchestrator                ⋯  │
│                                  │
│ ╭──────────────────────────────╮ │
│ │ [Questions] Checkout   1/2 ●○│ │
│ │                              │ │
│ │ BRANCH                       │ │
│ │ Base the fix on main or on   │ │
│ │ release/0.18?                │ │
│ │ ┌──────────────────────────┐ │ │
│ │ │◉ main        Recommended │ │ │
│ │ │  Ships in 0.18.2         │ │ │
│ │ └──────────────────────────┘ │ │
│ │ ┌──────────────────────────┐ │ │
│ │ │○ release/0.18            │ │ │
│ │ │  Hotfix now              │ │ │
│ │ └──────────────────────────┘ │ │
│ │  Type another answer…        │ │
│ │                              │ │
│ │ ┌──────────────────────────┐ │ │
│ │ │   Accept recommended     │ │ │   ← .ny-primary, 44px
│ │ └──────────────────────────┘ │ │
│ │   Skip            Next ›     │ │   ← .ny-quiet / .ny-link
│ ╰──────────────────────────────╯ │
│ [ composer …                  ]  │
└──────────────────────────────────┘
```

"Accept recommended" stays on every step and answers **all** questions (remaining ones as
recommended), so one tap still finishes. Uses the existing `@media (max-width: 767px)` and
`(pointer: coarse)` rules; no new breakpoints.

### 6.3 Needs you (`ui/gui-needs-you.js`)

A new card kind `questions` (`NEEDS_YOU_KINDS.questions = { label: "Questions", glyph: "needs",
rank: 0 }`), built from every tab with an open `view.questions` (including the Orchestrator's own,
which `needsYouItems()` otherwise skips), drawn with the same `gui-questions.js` renderer inside
`.ny-card`. Its `where` says who asks: "Worker · <Captain project> · MacBook", "Workflow · <name> ·
Mac mini", "Orchestrator". Answering calls `Call::AnswerQuestions` (not `answerAsk`, which is the
screen-menu path), so it works for headless children and the phone. Such a tab is filtered out of
the plain `question` card kind so it shows once.

```
╭ Needs you ───────────────────────────╮
│ ● Questions  site · Worker · MacBook │
│ Fix the checkout form                │
│ Base the fix on main or release?  1/2│
│ [◉ main  Recommended]  [○ release]   │
│ [ Accept recommended ]   Skip   Open │
╰──────────────────────────────────────╯
```

### 6.4 Where a parent shows its children's questions

* **Worker banner** (`ui/gui-worker-banner.js`): `ASK_WORDS.questions = "has questions"`; the
  banner's one-tap button is "Accept recommended" and "Open" goes to the worker. The parent
  (Captain or loose thread) sees it over its composer, as it does a worker's approval today.
* **Orchestrator chat strip**: above the Orchestrator composer, a one-line strip
  "Capitão site has questions · Answer" when a Captain or one of its workers in a project the
  Orchestrator sent to has an open questionnaire. "Answer" expands the same card in place. It reuses
  the worker banner's element and stepping ("1 of N").

## 7. Wire changes (all additive)

Rules from `AGENTS.md`: add variants, add `#[serde(default)]` fields, rename nothing, drop no
route. `wire-contract snapshot current` after each wire PR; `wire-contract check --base main`
must list no new break.

| Type (`crates/core/src/wire.rs` / `global_orchestrator.rs`) | Change | Who sends / reads | Old peers |
|---|---|---|---|
| `TabView` | `+ questions: Option<Questionnaire>` `#[serde(default, skip_serializing_if = "Option::is_none")]` | node → app, node → hub overview → other machines and hub web | ignore it; still see `status: needs-input`, `message: "Claude has questions"` and answer by opening the thread (which shows the tool call as a pending tool step) |
| `Questionnaire` (new struct) | `{ id, title?, questions: Vec<Question>, projects: Vec<String> #[default], at: u64 }`; `Question { header?, question, changes, options: Vec<QOption>, multi_select #[default] }`; `QOption { label, description?, recommended #[default] }` | inside `TabView` | — |
| `Call` | `+ AnswerQuestions { tab: TabId, id: String, reply: QuestionsReply }` | app / hub web → node (relayed by the hub like any `Call`) | an old node answers "unknown variant" → the app falls back to Open |
| `QuestionsReply` (new enum, `#[serde(tag = "kind", rename_all = "snake_case")]`) | `Answered { answers: Vec<Answer> }`, `Skipped` | inside `Call::AnswerQuestions` | — |
| `Answer` | `{ chosen: Vec<String> /* labels */, text: Option<String> }`, positional with the questions | inside the reply | — |
| `Waiting` | `+ questions: Option<String>` (the questionnaire id) `#[serde(default, skip_serializing_if)]`. **No new `AskKind` variant**: `ask` stays `question`, because an older node deserialising the hub's overview would fail on an unknown `AskKind` | hub overview → node's `waiting()` | ignore it |
| `ProjectTool::QuestionUser { .. }`, `ProjectTool::QuestionStatus { id }` (`crates/core/src/agent.rs:70`) | new variants | MCP child → its own node over the local socket, same binary | not on the wire contract (local socket only) |

`Call::AnswerQuestions` also gets its line in the agent surface manifest
(`crates/core/src/surface.rs`, next to `write_input`) as **`Excluded`**: "only the user answers an
agent's questions". An agent can never answer another agent's questionnaire through `act`.

No hub route changes; no DB migration (questionnaires live on the node, persisted in its config
dir). The hub only relays the `Call` and carries `TabView` as it already does.

## 8. PR split, proof, model tier

Each PR is small enough to review in one sitting, lands behind nothing (the tool is useless until
PR 2 offers it, harmless before), and runs `cargo run -p pinned-server --bin wire-contract -- check
--base main` clean (PRs 1 and 4 also run `snapshot current`).

| # | PR | Scope | Proof | Tier |
|---|---|---|---|---|
| 1 | `wire: questionnaires on tabs` | `crates/core/src/questions.rs` (store, persisted `questions.json`); `Questionnaire`/`Question`/`QOption`/`QuestionsReply`/`Answer`; `TabView.questions`; `Call::AnswerQuestions` on `Machine::call`; view override to mid-turn `needs-input` (§3.1); `Waiting.questions`; surface manifest `Excluded`; `snapshot current` | serde round-trips in `wire.rs` tests (old JSON without the field reads; a `Call::AnswerQuestions` round-trip); `waiting_kind`/`ask_of` on a tab with questions; `waiting_cases.json` gains the case so `ui/global-chat.js` and Rust agree; reload test; `note_user_waits` pauses a task's clock | **opus** (wire design, back-compat) |
| 2 | `mcp: question_user for every agent` | tool definition (`Audience::Everyone`), validation (§2.1), blocking poll + progress, `pending` + notice fallback, `StopTurn`/close/archive cancel, delegated-child routing via `task`, "first finished turn with no open questionnaire", `MCP_TOOL_TIMEOUT` in session env, `permissions()` | `reminders.rs` over-socket tests: blocks, progresses, returns `answered`/`skipped`/`cancelled`/`pending`; offered to all six seats; refused shapes (recommended not first, two recommended, 5 questions); `machine.rs` test that a worker's turn ending with an open questionnaire does not end its task | **opus** |
| 3 | `orchestrator: ask before send_to_captain` | gate in `project_tool` (§4.2), `projects` required for the global seat, prompt rule 2b and the Opções note, `captain_prompt` sentence | tests 1–8 of §4.3, all in `cargo test -p pinned-core`; the PR body pastes the refusals' exact text | **opus** (enforcement and prompt wording) |
| 4 | `hub: questions keep a box and relay answers` | `Hub::seen`/`seen_any` treat a tab with questions as not quiet; `crates/server/tests/hub.rs` relay of `Call::AnswerQuestions` to another machine and `waiting()` listing a Captain's questionnaire | `boxes.rs` verdict test (a questioning tab never idles); the two hub tests | **sonnet** |
| 5 | `ui: questionnaire card in the conversation` | `ui/gui-questions.js` + `.css` (renderer, pre-selected recommendation, Accept recommended, Send, typed answers, multi-select, Skip with its warning), `shownHtml` branch and answered row in `gui-timeline.js`, composer placeholder, phone stepper | `ui/questions.test.js` (node vm, like `ui/needs-you.test.js`): renders, one-tap accept sends `answer_questions` with every recommended label, a typed answer, skip needs the second tap and sends `skipped`, a stale card shows the "missed" note; screenshots at 1280 px and 390 px, light and dark | **sonnet** |
| 6 | `ui: questions in Needs you, the worker banner and the Orchestrator strip` | `NEEDS_YOU_KINDS.questions`, card from `view.questions` (Orchestrator included, plain `question` card deduped), worker banner "has questions" + Accept recommended, Orchestrator strip | `ui/needs-you.test.js` and `ui/worker-banner.test.js` (a worker's and a workflow run's questionnaire answered from Needs you; one card per tab); screenshots at 390 px | **sonnet** |
| 7 | `proof: tutorial video and changelog 0.18.2` | recorded with the `pinned:tutorial-video` skill (visible clicks through the sidebar and Needs you, no URL jumps), attached with `pinned evidence`; CHANGELOG 0.18.2 entries | one video, three chapters: **answer** (Orchestrator asks two questions, user picks the non-recommended option and types a note, one complete `[Orchestrator]` message reaches the Captain, no second message); **accept recommendation** (a worker under a Captain asks; answered with one tap from Needs you at phone width on hub web; the worker goes on); **skip** (a workflow run asks; skip shows its warning; the run goes on and its reply lists "Suposições/Assumptions"). Plus a shot of `send_to_captain` refused while open | **sonnet** |

Order: 1 → 2 → 3 (core; 3 can be reviewed in parallel with 2 against 1), then 4, 5, 6 in parallel,
then 7 on the release candidate. Ship as 0.18.2 only when 1–6 are merged; 1 alone is wire-safe to
ship early.

## 9. Open points for Felipe

1. **Worker questions go straight to you** (§3.2), not through the Captain and the Orchestrator.
   The Captain sees them in its worker banner. Say if you want `to: "parent"` routing later.
2. **Unattended runs wait forever by default** (§5); a per-workflow "go on with its
   recommendation" switch is left for later.
3. **Opções: stays** for end-of-turn offers; `question_user` is for anything that decides what is
   sent or done.
4. **The hub's external MCP** (`crates/server/src/mcp.rs`) does not get the tool in 0.18.2.
