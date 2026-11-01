# Watched thread nudge

Completed 2026-10-31. Reviews [34 round 4](https://github.com/FelipeGazapina/pinned/pull/34#issuecomment-6546439314) and [round 5](https://github.com/FelipeGazapina/pinned/pull/34#issuecomment-6549013060).

## One read path

The machine sends `{"kind": "thread", "tab": ...}` to each client that watches a tab when that tab's session, transcript path, or transcript length differs from the last look. It also sends one on the first look. The event carries no lines. The page reads them with `thread`, the same call as the 1200 ms poll, so the poll's page is the only page.

The tab keeps the transcript path it last found. Reads, nudges and forks all take it from `Machine::kept_transcript`, so they always use the same copy. A search of the project folders runs again only when that file is gone, or when the tab's session or account changed. The path is advisory, so a tab that closes during a search still gets its read.

`fetchThread` in `ui/gui.js` is the only writer of a thread's cursor. A nudge or a poll tick during a read sets `again`, and the read runs once more. The earlier push moved the cursor from a second path. A poll in flight then came back stale and was dropped, with the `agents` list that only the poll carried.

A read repaints when `drawnOf` changes. It holds every thread field the screen draws: the session, loaded, unsupported, error, the item count, the tasks, the context, the branch, and the agents key. A reset always repaints, and it counts in `renderThread`'s content key, so a rewind that leaves as many lines as before still draws the new last line. A reset or a new session clears the context ring along with the lines, the tasks, and the branch.

## What the page watches

`followed()` in `ui/gui.js` names the tabs the page watches, the tabs the 1200 ms poll reads, and the tabs a nudge reads. The three sets share that one definition, but each reads it at a different time. The node nudges the union of every viewer's watch, and Tauri forwards each event to the desktop page, so `onThreadEvent` also checks that the page is visible and in app mode. The node sends one nudge for each change, so a nudge dropped there is gone. The page reads every followed tab when it becomes visible again and when it returns to app mode. `sentWatch` keeps the watch each machine was last sent. The page sends only to machines whose tabs changed, and it forgets a machine that left the page, as on sign-out. A failed send restores that one machine's entry, so the next render sends it again.

The hub forwards a client's watch to the machine and keeps it to filter events, to unwatch on `client_gone`, and to replay in `came_online`. Usually the page sees the machine go offline, drops its tabs, sends an empty watch, and watches again once the tabs return. The replay covers the other order. The old socket's close handler removes the machine and then awaits the database while the new socket registers it, so the page sees the machine online throughout and never sends its watch again.

Reads of one tab start at least `THREAD_READ_GAP_MS` (100 ms) apart. Over the internet a read takes longer than that, so only a nearby page waits. A nudge after a quiet spell reads at once.

## Machine loops

The watcher and the pull request loop run through `Machine::every`. Each lives as long as the machine and skips its ticks while the machine quits. A failed self-update calls `quit`, then `reopen_sessions` on the same process, and the loops serve again.

The first watch starts the watcher, from this machine's page or through the hub. A tick with nothing watched returns before it takes the tabs lock. That return also skips the cleanup of closed tabs from the watcher's state, which the next tick with a watch does.

A tab can name a session before Claude writes its transcript. A search of the project folders that finds nothing waits 40 ms, then twice as long each time, up to `PAGE_POLL`. A new session or a new status starts over at 40 ms.

## Remote folders

`open_tab` rejects a path that is not a folder from this branch on. `openRemote` sends the path in `open_tab` only to a machine whose version is newer than `NEWEST_RELEASE_THAT_OPENS_ANY_PATH` (0.16.0). Older nodes, and nodes that name no version, get a fresh tab moved with `set_folder`, which closes when the move fails. If a release ships before this branch, raise the constant to that release.

## Tab list wait

`whenListed` in `ui/main.js` waits up to 2 seconds for the tab list after `open_tab`. Every release since v0.8.2 emits that list inside `open_tab` before it answers, so the timer runs only when the event is lost. Asking at once would fetch a list the push already covered, which `ui/reveal-tab.test.js` forbids.

## Evidence

Three alternating runs per side of `ITERATIONS=40 RTT_MS=150 scripts/latency-bench/run.sh`, errors 0. `thread_message` p50 476 ms (475 to 483) on the nudge against 883 ms (882 to 884) on `origin/main` at `85ef28b`, 1.86x. p95 507 ms against 1288 ms. The nudge costs one more round trip than the earlier page push, which measured 171 to 179 ms. `wire-contract check --base origin/main` reported no new breaks.

Round 5, after the merge with `origin/main` at `327708e`. Three alternating runs per side of the same bench, errors 0 and 240 work items in every run. `thread_message` p50 481 ms (477 to 482) against 886 ms (880 to 892) on main, 1.84x. p95 506 ms against 1289 ms. The round 4 head measured 480 ms (478 to 484) in the same rotation. `write_input` stayed at 154 ms on every side, and `open_remote` read 517 ms on this branch against 529 ms on main. In headless Chromium at 0 ms RTT, a transcript that gained a line every 11 ms was read 10 times a second, against 22 before the read gap.
