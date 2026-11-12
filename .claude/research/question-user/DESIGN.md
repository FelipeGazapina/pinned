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
taste chat, plain thread, worker thread and delegated child (`child == true`). It is the first
pinned tool a delegated child is offered from `project_tools()`, so `call()` must route it before
`project_request` demands `PINNED_TAB` (`in_tab`, `reminders.rs:867`): a child sends
`Message::Project` with `task` set instead of `tab`, and the node resolves the child's display tab.

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
`Working`): the view is computed as "open questionnaire ⇒ needs-input", the same way `interrupted`
already overrides status. `waiting_kind()` keeps returning `None` for `orchestrator_global`; its own
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
**before** any routing, resolving `project` to its key the same way `projects()` lists it:

```rust
ProjectTool::SendToCaptain { project, .. } if let Some(open) = self.questions_holding(tab, &project) =>
    Err(format!("questions {} are open in this conversation about {}: wait for the answers or the skip, then send one complete message", open.id, open.project_name)),
```

The gate keys on the **resolved** project (repo key), so `site`, `github.com/o/site` and
`m-book:/w/site` all hit it. A questionnaire without `projects` cannot come from the Orchestrator
(validation refuses it), so the gate never blocks everything by accident, and other projects in the
same user message go out.

`question_user` itself, for the Orchestrator, also refuses a project that already has an in-flight
request from this conversation in the ledger (`Ledger::in_flight`): "the Capitão of site already
works on r-3; ask before you send, not after". That turns "send, then ask, then steer" into an error
the model sees, which is the behaviour Felipe described.

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
6. A second `question_user` while one is open is refused; a project already in flight is refused.
7. The gate survives a reload of `questions.json` (new `Machine` on the same config dir).
8. `global_orchestrator.rs`: the prompt names `question_user` before `send_to_captain`, the gate
   and "Suposições"; the `captain_prompt` test (`:1193`) gets the new sentence.
9. `reminders.rs`: `question_user` is offered to every seat including a child (extend
   `a_session_gets_the_project_tools_a_delegated_child_lacks` `:1256` and the global list at `:1444`);
   an over-socket test like `a_wait_returns_the_summary…` (`:1058`) shows the call blocks, sends
   progress, and returns `answered` / `skipped` / `pending` (short `PINNED_QUESTION_WAIT_MS`).
10. `crates/server/tests/hub.rs`: an answer sent from a hub client to a tab on another machine
    reaches it (relay), and the Orchestrator's `waiting()` lists the Captain's open questionnaire.

## 5. No user there: scheduled and workflow runs
_(in progress)_

## 6. UI (desktop and hub web, phone width)
_(in progress)_

## 7. Wire changes (additive)
_(in progress)_

## 8. PR split, proof, model tier
_(in progress)_
