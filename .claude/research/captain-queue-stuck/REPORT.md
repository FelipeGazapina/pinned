# Orchestrator messages stuck in a Captain's queue (0.18.2)

Bug seen on 2026-11-12. Between 17:38 and 22:25 UTC the global Orchestrator sent several messages
to pinned's Captain, and none arrived. The Captain's turn had ended at about 17:38, but the
Orchestrator saw it as `trabalhando` (working) the whole time. The messages arrived together only
after the user typed "read your queue", and they came in as UserPromptSubmit and Stop hook
"additional context" instead of as turns.

Line numbers below are on branch `fix/captain-queue-stuck` unless marked `main`.

## Cause

A Stop hook hand-over that never reached Claude Code. pinned counted it delivered, so it marked
the Captain busy (`turn_pending`) with nothing left that would clear it.

1. The first message arrived at about 17:38, while the Captain was finishing its turn. The tab
   was busy, so `hand_to` queued it with the tab's news
   (`crates/core/src/machine.rs:2947`, `delegates.queue_request`).
2. The turn's Stop hook connects to the machine and asks for anything to hand over. The machine:
   - claims the whole queue (`take_notices`, `machine.rs:4330` → `machine.rs:6095`) and drops the
     claimed requests (`drop_claimed_reports`, `machine.rs:6102`);
   - marks those messages in flight (`mark_typed`, `machine.rs:4334` → `Ledger::typed`,
     `global_orchestrator.rs:215`);
   - sets `turn_pending = now` (`machine.rs:4337`), on the assumption that the Stop hook's
     `additionalContext` continues the turn and its next Stop will clear `turn_pending`;
   - only then answers with the text.
3. The hook process waits for that answer for **300 ms** (`RECALL_WAIT`), the same short budget a
   *prompt* gets for memory recall (`crates/core/src/hooks.rs:31`, and on `main` at
   `hooks.rs:226` `set_read_timeout(Some(RECALL_WAIT))`). Before the machine answers a Captain's
   Stop, `record_event` runs synchronously. That includes `record_turn` (`machine.rs:5949`), which
   starts **two `git diff` processes**, one of them with `--find-renames`
   (`crates/core/src/history.rs:129`, `:134`), and appends to the history file. It also includes
   `save_sessions` and `emit_tabs`. On a Mac that is also running the Captain's background shells
   (cargo builds) and its delegated workers, this easily takes longer than 300 ms. When it does,
   the hook stops waiting, prints nothing, and Claude Code ends the turn normally. The machine has
   already committed every step of point 2.
4. The result is a Captain whose `status` is `Done` but whose `turn_pending` is set, with no turn
   running.

### Which state made the node consider the tab busy

`Tab::turn_pending` (`machine.rs:591`, `notice_idle()` returns false while it is `Some`).
`notice_idle` gates every way a queued message reaches the session: `hand_to` decides
Idle/Busy with it (`machine.rs:2943`), `write_notice` checks it, and so do `maybe_deliver` /
`parent_idle` (`machine.rs:5630`) and the delegate pass that calls `maybe_deliver` every second.
Every later `send_to_captain` was therefore answered `Queued` and added to the same queue.

The Orchestrator's own view of the Captain: `CaptainState::of` returns `Working` when
`tab.orchestrator_turn` is true (`global_orchestrator.rs:405`). `orchestrator_turn` is true while
the ledger has an in-flight message for the project (`machine.rs:551`). Step 2 had marked the
hand-over's message in flight, and only the end of a Captain turn answers it
(`captain_turn_ended`, `machine.rs:3256`). No turn ended until the user typed, so `projects()`
showed `trabalhando` for five hours.

### Why nothing woke the delivery once the Captain was idle

Only three things clear `turn_pending`: a Done or Failed hook (`record_status`, `machine.rs:6176`),
a session (re)start, and Claude Code taking held text. The Stop that would have cleared it was the
one that had just run. The continuation turn it expected never started, so no later Stop came.
Claude Code does say the session is idle: its `Notification` hook ("Claude is waiting for your
input") fires about a minute after a turn ends. But `record_status` only recorded that as
`NeedsInput` and left `turn_pending` alone, and nothing has a timeout on `turn_pending`.

### Why the messages arrived as hook context, not as turns

The user's own prompt fired UserPromptSubmit with `hook_feedback`. `take_notices` claimed the whole
queue, and the hook printed it as `additionalContext` (`hooks.rs:235`). Whatever arrived during
that turn went out in its Stop hook the same way. That is the normal path for news that arrives
while a turn runs. The bug is only that no turn ran for five hours.

## Evidence that the hand-over is lost on pinned's side, not in Claude Code

- Claude Code (the 2.1.293 bundle in this box, `claude-agent-sdk-linux-arm64/claude`) does honor
  a Stop hook's `additionalContext`. In the Stop handler,
  `if(pe.additionalContexts&&pe.additionalContexts.length>0){… if(!$yn(he))ge.push(he) …}` puts the
  attachment into the list of blocking messages, and `$yn` returns false unless the session runs
  `--bare` (`function $yn(e){if(!br())return!1;…}`; this `br` is imported with the safe-mode and
  restricted-mode helpers and is `CLAUDE_CODE_SIMPLE || --bare`). The query loop then continues with
  `stopHookActive:true` (`if(Je.blockingErrors.length>0){… transition:{reason:"stop_hook_blocking"}}`).
  The next Stop has `stop_hook_active: true`, and pinned's hook then only reports Done
  (`hooks.rs:221`), which clears `turn_pending`. So if pinned's reply reaches Claude Code, the turn
  continues and the state clears itself. A `turn_pending` that stays set means the reply never
  arrived.
- The same day, messages sent while the Captain was idle were typed as turns, and Stop hook
  hand-overs that arrived in time continued their turns. The failure needs a message to be queued
  at the moment a turn ends, plus a slow reply. That matches the first message at 17:38, the same
  minute the turn ended.
- Reproduced in a unit test (below). With today's code, a hand-over Claude Code never received
  leaves the next queued notice untyped even after Claude Code's idle notice.

### What the owner's logs can confirm

If the cause is right, the Captain's transcript from about 17:38 has no "Stop hook additional
context" carrying the 17:38 message. That message (and any delegated-task notice claimed with it)
was claimed and then lost, so it should not appear among the messages read after "read your queue"
either. The Orchestrator's ledger (`sent.json`) shows it as `answered` only after the user's turn.

## Other hypotheses, and why they don't fit

| Hypothesis | Verdict |
| --- | --- |
| Claude Code ignores Stop `additionalContext`, so the turn never continues | Ruled out for non-`--bare` sessions by the bundle code above. It would also have broken every Stop hand-over, not one evening. |
| `status` stuck at `Working` (a UserPromptSubmit with no Stop, e.g. a `<task-notification>` turn) | Claude Code ends every such turn with a Stop. `read_captain` would have shown `working` too; the report only says the overview did. Nothing in the code leaves a Captain `Working` after its Stop: a Captain is never held with `background_wait` (`holds` covers only workers and role-less tasks, `machine.rs:5923`). |
| `Failed` (StopFailure) leaves `notice_idle()` false | `captain_turn_ended` would have reported "Parou com um erro" to the Orchestrator and answered the ledger. `CaptainState` would then be `livre`, not `trabalhando`. |
| A non-empty `draft` (unsent text, or Up/Down recalling a prompt) blocks typing | Possible in principle, but on its own it would show the Captain as `livre`. It needs a second, unrelated in-flight message to look `trabalhando`. The user did not report leaving text in the Captain. |
| PR #167's `CAPTAIN_HOLD` leaves the ledger in flight while delegated tasks run | It can make the overview say `trabalhando`, because the 10-minute cap is only checked at the next turn end. It does not block delivery: `hand_to` types into an idle tab whatever the ledger says. It is a separate latent issue (see Follow-ups). |

## Fix (implemented on this branch)

1. **The Stop hook waits for the hand-over.** `feed` waits up to `HAND_OVER_WAIT` = 10 s for a
   Stop's reply and keeps the 300 ms `RECALL_WAIT` for prompts (`hooks.rs:35`, `:230`). A slow
   reply only delays the end of the turn. It never delays the user's typing, and Claude Code's own
   hook timeout is far longer.
2. **Claude Code's idle notice ends a stale `turn_pending`.** In `record_status`, a
   "waiting for your input" notice on a `Done` tab clears a `turn_pending` older than
   `IDLE_NOTICE_GRACE` = 10 s (`machine.rs:87`, `:6172`). `record_event` then calls
   `maybe_deliver`, which types the queue as a new turn. So a hand-over lost for any reason, or a
   typed prompt that never submitted, costs about a minute instead of hours. The new turn's end
   answers the stale in-flight messages, so the overview goes back to `livre`.

The check requires the exact idle wording and a `Done` tab. A permission prompt raised inside a
Stop-hook continuation (status still `Done`, no UserPromptSubmit) therefore never clears
`turn_pending`, and nothing is typed into its menu. The grace period keeps an idle notice that was
already on its way from cancelling a prompt pinned typed a moment earlier.

## Tests (failing on `main`, passing here)

- `machine::tests::a_hand_over_claude_code_never_got_does_not_strand_the_queue`
  (`machine.rs`, after `a_stop_hook_feeds_a_notice_the_draft_would_block`). A Stop hook hands one
  notice over and Claude Code never gets it. A second notice stays queued, an idle notice right
  after does not type it, and an idle notice a minute later does. On `main` it fails with
  "timed out waiting for the queue after the idle notification".
- `hooks::tests::a_stop_hook_waits_for_a_slow_hand_over`. The machine answers a Stop after 1 s. On
  `main`, `deliver` returns `None` and the hand-over is lost.

Ran `cargo test -p pinned-core --lib`: 1201 passed, 2 failed. The two failures,
`a_report_starts_the_global_orchestrator_on_its_line_once` and
`a_message_from_the_global_orchestrator_comes_back_as_the_capitaos_report_when_its_turn_ends`,
fail the same way on unmodified `main` in this box. The recorded launch picks up this box's own
pinned session environment (the third argument is the box's checkout instead of `""`), so they are
not caused by this change. `wire-contract check --base main`: no new breaks, exit 0. No wire types
changed. Clippy and rustfmt report nothing in the changed lines.

## Risk

- **Low.** Both changes only turn a stuck state into a delivered one. A Stop hook can now take up
  to 10 s longer to return when the machine is slow; normally it answers in milliseconds.
- **Residual: the text of a lost hand-over is still lost.** The machine drops what it claims before
  it knows the hook printed it. With a 10 s budget that needs a very slow machine. A full fix needs
  an acknowledgement, for example releasing the claim unless the continuation's
  `stop_hook_active` Stop arrives, or confirming the hand-over in the transcript.
- **Residual: UserPromptSubmit has the same claim-before-answer pattern** with its 300 ms budget.
  A late answer there loses the notices, but a Stop always follows, so it never strands the queue.

## Follow-ups (not in this PR)

1. Acknowledge hand-overs, so a reply the hook never printed is put back in the queue (see Risk).
2. Move `record_turn`'s `git diff` work, and anything else slow, off the hook's reply path. The
   reply only needs the claimed notices.
3. Check `CAPTAIN_HOLD` on a timer, not only at the next turn end, so a held report can't keep
   the overview at `trabalhando` past its 10-minute cap.
