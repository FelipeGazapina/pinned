# Watched thread nudge

Completed 2026-10-31. Review [34](https://github.com/FelipeGazapina/pinned/pull/34#issuecomment-6546439314).

## One read path

The machine sends `{"kind": "thread", "tab": ...}` to each client that watches a tab when that tab's session, transcript path, or transcript length differs from the last look. It also sends one on the first look. The event carries no lines. The page reads them with `thread`, the same call as the 1200 ms poll, so the poll's page is the only page.

`fetchThread` in `ui/gui.js` is the only writer of a thread's cursor. A nudge or a poll tick during a read sets `again`, and the read runs once more. The earlier push moved the cursor from a second path. A poll in flight then came back stale and was dropped, with the `agents` list that only the poll carried.

A read repaints when `drawnOf` changes. It holds every thread field the screen draws: the session, loaded, unsupported, error, the item count, the tasks, the context, the branch, and the agents key. A reset always repaints. A reset or a new session clears the context ring along with the lines, the tasks, and the branch.

## Tab list wait

`whenListed` in `ui/main.js` waits up to 2 seconds for the tab list after `open_tab`. Every release since v0.8.2 emits that list inside `open_tab` before it answers, so the timer runs only when the event is lost. Asking at once would fetch a list the push already covered, which `ui/reveal-tab.test.js` forbids.

## Evidence

Three alternating runs per side of `ITERATIONS=40 RTT_MS=150 scripts/latency-bench/run.sh`, errors 0. `thread_message` p50 476 ms (475 to 483) on the nudge against 883 ms (882 to 884) on `origin/main` at `85ef28b`, 1.86x. p95 507 ms against 1288 ms. The nudge costs one more round trip than the earlier page push, which measured 171 to 179 ms. `wire-contract check --base origin/main` reported no new breaks.
