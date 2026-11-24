# 0.18.7 agent team: contracts and file ownership (C0)

C0 landed every shared type, wire message, tool schema and hook for U2 to U13, all inert. A unit
fills in behaviour inside its own files. When it has to touch a shared file, it changes only the
lines named here.

## What C0 landed, and how each entry point behaves until its unit fills it in

| Entry point | Where | Until its unit |
|---|---|---|
| `Tastes.reviewer` (`Reviewer`, `ProjectReview`, `Switch`, `Rule`) | `taste.rs`, `reviewer.rs` | Stored and read. Default rounds 2, range 1 to 5 |
| `Tastes.worker_profiles` (`WorkerProfiles`, `WorkerProfile`, `ProfileAccess`, `ProfileAuthor`) | `taste.rs`, `profiles.rs` | Stored and read |
| `HubCall::SetReviewer { patch }` | `wire.rs`, `server/src/hub.rs` | Real store via `Tastes::set_reviewer`. `Excluded` in `surface.rs` |
| `HubCall::SetWorkerProfiles { project, profiles, drop_proposed }` | `wire.rs`, `server/src/hub.rs` | Real store via `Tastes::set_worker_profiles`, then `Tastes::drop_proposed` in the same write. Approve sends the list with the row and `drop_proposed: [name]`; Discard sends the list unchanged and `drop_proposed: [name]`. `Excluded`. A machine may not forward it |
| `HubCall::ProposeWorkerProfile { tab, profile }` | `wire.rs`, `taste.rs` (`Tastes::propose_worker_profile`, `MAX_PROPOSED`), `server/src/hub.rs` (`Hub::propose_worker_profile`), `server/src/http.rs` | Real store, into `proposed` only. From a machine only, for a `tab` that machine reports as a Captain or the global orchestrator; the app's own call and every other tab are refused. The hub trims the name, writes `by: orchestrator`, replaces a proposal of the same name and refuses a 21st. `Excluded` in `surface.rs`; `Gate::Never` on a team |
| `Machine::propose_worker_profile(tab, profile)` | `machine/propose.rs` | Real: sends `ProposeWorkerProfile` while linked, else writes this machine's own tastes. Nothing calls it until U5's `profile_propose` does |
| `Tastes::check` | `taste.rs` | Also runs `Reviewer::check` and `WorkerProfile::check` on every list (account, each project, proposed), with no two names alike in a list and at most `MAX_PROPOSED` proposed, so `ImportTastes` refuses what the user's own writes refuse |
| `taste_set`, `set_taste` with `reviewer` or `worker_profiles` in the patch | `taste::patch_without_user_only` | Refused: "… is the user's own, set only on the Way of working page; nothing changed" |
| `taste_get` | `taste::ScopeView` | Also returns `reviewer` and `worker_profiles`, read-only |
| `HubCall::DecideConfirmation.reason` | `wire.rs`, `server/src/surface.rs::decide(_reason)` | Parsed and carried. Not shown |
| `Call::Board { tab, task }` (`board` action, node Read) | `wire.rs`, `machine/board.rs` | Answers an empty `BoardView` |
| `delegate_task` `profile`, `then` | `delegate::Request`, `profiles::admit`, `chain::admit` | Refused on both the MCP side and the machine side, in `start_delegate` and in `start_for_origin` (a worker another machine sent): "profile is not available in this build", "then is not available in this build". Starts nothing |
| Task record `reviews`, `profile`, `chain` | `delegate/mod.rs::Task`, `registry.rs` (`put_review`, `set_profile`, `set_chain`) | Saved and loaded. `task_status` answers `reviews` when there are any |
| `Role::Reviewer` | `agent.rs` | Disallows Edit, Write, NotebookEdit, Bash. `Machine::review_turn_end` starts one as a read-only thread under the worker, on the task's change and the rules. On the wire as `TabView.role: "reviewer"`: no app's Rust reads another machine's `TabView` (`ToMachine` carries none; `ToClient` is read by the UI's JavaScript, which shows an unknown role as a plain thread), so old apps take it |
| Team tools `task_fix`, `feature_map_read`, `feature_map_set`, `profile_propose`, `board_note`, `board_read`, `ask_worker` | `team_tools.rs` (args, schemas, `*_offered`) | No session is offered them. A call reaches `Machine::team_tool` through `Message::Team` and answers "<tool> is not available in this build" |
| `Machine::review_turn_end(task, tab) -> bool` | `machine/reviewer.rs`, called from `settle_thread_task` | Real for one round (U8): a worker thread with no origin, in a project whose switch is `on` with a rule and with a non-empty change since the task's `base`, gets a reviewer and true. The task ends when the reviewer's turn does, with a `passed` or `limit` round and the findings in its notice; any failure, a closed reviewer or a restart ends it with "its review did not run: …". Send-back stays U9's |
| `Machine::send_back(task, round, SendBackBy)` | `machine/send_back.rs` | Returns an error |
| `Machine::board_write(task, BoardEvent)` | `machine/board.rs`, called after a worker's task settles | Does nothing |
| `Machine::chain_settled(task)` | `machine/chain.rs`, called after `board_write` | Does nothing |
| `Machine::profile_request`, `Machine::chain_request` | `machine/profiles.rs`, `machine/chain.rs`, called from `start_delegate`, and from `start_for_origin` with the caller `tab:` the origin's parent key | Refuse `profile` and `then` |
| `Machine::delegate_started(task, request)` | `machine/delegate_started.rs`, called once the task exists: after `start_task` in `start_delegate`, after `launch_child` in `start_for_origin`. A replayed request calls it again with the same task | Does nothing. U6 adds `Delegates::set_profile`, U12 `Delegates::set_chain` |
| `delegate_task` schema properties | `profiles::delegate_property`, `chain::delegate_property` | Return None, so the schema is unchanged |
| Way of working sections | `ui/gui-reviewer.js` (`tasteReviewerHtml`, `TASTE_REVIEWER_ACTS`), `ui/gui-profiles.js` (`tasteProfilesHtml`, `TASTE_PROFILE_ACTS`) | Draw nothing |

## Ownership map

"Turns on" means the unit edits only its own `*_offered` function in `team_tools.rs`, or only its
own `delegate_property` function. Those are separate functions, so the edits merge cleanly.

| Unit | Edits | Must not edit | Shares with |
|---|---|---|---|
| U2 deny reason | `ui/agent-surface.js`, `ui/gui-needs-you.js`, `server/src/surface.rs` (`decide` body: deliver `_reason` to the session) | `wire.rs` (the field exists), `machine.rs`, `taste.rs` | none |
| U3 feature map | `core/src/feature_map.rs`, `core/src/machine/feature_map.rs`, `team_tools.rs` (only `feature_map_read_offered`, `feature_map_set_offered`, and their arms in `definition`) | `machine.rs`, `agent.rs`, `reminders.rs` | `team_tools.rs` (own lines only) |
| U4 reviewer store | `core/src/reviewer.rs` (store and checks), `src/main.rs` (signed-out `set_reviewer` Tauri command calling `Machine::set_local_reviewer`) | `taste.rs`, `wire.rs`, `surface.rs`, `machine/reviewer.rs` (its part is done) | `src/main.rs` with U5: different commands, so they merge |
| U5 profiles | `core/src/profiles.rs` (lists and checks), `core/src/machine/profiles.rs` (`profile_propose`: checks the caller is a Captain or the global orchestrator, then calls `Machine::propose_worker_profile` in `machine/propose.rs`), `team_tools.rs` (only `profile_propose_offered` and its arm), `src/main.rs` (`set_worker_profiles` command, with `drop_proposed` for Approve and Discard) | `taste.rs`, `wire.rs`, `surface.rs`, `machine/propose.rs` | `profiles.rs` and `machine/profiles.rs` with U6: run U5 then U6 |
| U6 delegate with profile | `core/src/profiles.rs` (`admit`, `delegate_property`), `core/src/machine/profiles.rs` (`profile_request`), `core/src/machine/delegate_started.rs` (one `Delegates::set_profile` line) | `delegate/*`, `machine.rs`, `reminders.rs` | as above; `delegate_started.rs` with U12 |
| U7a Way of working: reviewer | `ui/gui-reviewer.js`, `ui/gui-taste.css` | `ui/gui-taste.js` (the hooks exist) | `gui-taste.css` with U7b, and `ui/taste.js` if needed: run U7a then U7b |
| U7b Way of working: profiles | `ui/gui-profiles.js`, `ui/gui-taste.css` | `ui/gui-taste.js` | as above |
| U8 review run | `core/src/machine/reviewer.rs` (`review_turn_end`), a new `core/src/review_run.rs` (brief, findings parse), `agent.rs` (only the `REVIEWER_PROMPT` text) | `machine.rs`, `delegate/*` (use `Delegates::put_review`), `reviewer.rs` | none |
| U9 send-back and `task_fix` | `core/src/machine/send_back.rs`, `team_tools.rs` (only `task_fix_offered` and its arm) | `machine.rs`, `machine/reviewer.rs` | none |
| U10 thread render | `ui/gui-timeline.js` (review rounds in the thread), a new `ui/gui-review-card.js` if needed | the other `ui/gui-*` files | `ui/gui-timeline.js` with nobody else in 0.18.7 |
| U11 board | `core/src/board.rs`, `core/src/machine/board.rs` (`board`, `board_write`, `board_note`, `board_read`), `team_tools.rs` (only the `board_*_offered` functions and their arms) | `wire.rs` (`Call::Board` exists), `machine.rs` | none |
| U12 chain | `core/src/chain.rs`, `core/src/machine/chain.rs` (`chain_request`, `chain_settled`, `ask_worker`), `core/src/machine/delegate_started.rs` (one `Delegates::set_chain` line), `team_tools.rs` (only `ask_worker_offered` and its arm) | `machine.rs`, `delegate/*` (use `Delegates::set_chain`), `machine/send_back.rs` (call `send_back`) | `delegate_started.rs` with U6; it calls U9's `send_back` |
| U13 board UI | new `ui/gui-board.js`, footer in `ui/gui-plan.js` (not in this tree yet: it arrives with #349's UI unit) | `ui/gui-taste.js`, `ui/gui-timeline.js` | `ui/gui-plan.js` with #349's UI unit: run after it |

Files no unit should need to touch: `machine.rs`, `wire.rs`, `surface.rs`, `taste.rs`,
`reminders.rs`, `delegate/*`, `agent.rs` (except U8's prompt text), `machine/team.rs` and
`ui/gui-taste.js`. If a unit still has to, it says so in its pull request. The files then
merge in sequence.

`ui/index.html` loads only `gui-reviewer.js` and `gui-profiles.js`. U13 adds its own
`<script src="gui-board.js">` line, and U10 adds one if it makes a new file.

## Pairs that run in sequence

- U5 then U6 (`profiles.rs`, `machine/profiles.rs`)
- U7a then U7b (`gui-taste.css`, possibly `taste.js`)
- U9 then U12, only for the call into `send_back`; the files are separate
- U6 then U12 in `machine/delegate_started.rs`, one line each in `delegate_started`
- U13 after #349's plan UI (`gui-plan.js`)
