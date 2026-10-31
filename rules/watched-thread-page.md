# Watched thread page

Completed 2026-10-31. Review [34](https://github.com/FelipeGazapina/pinned/pull/34#issuecomment-6545598281).

## Thread page

`TranscriptState::from_page` is the one read of a transcript. `Machine::thread` and `push_transcript_lines` both use it. The push event flattens that state, so it carries `context`, `branch`, and `todos` with `#[serde(default)]`. The phone applies a push through `takeThreadPage`, the same reducer as a poll. Agents stay on the poll.

A file that disappears for one tick keeps its cursor. A rejected `watch_threads` restores the previous signature, and a later hello wins over that rejection.

## Tab list wait

`open_tab` emits the tab list before it answers. A push that arrives first clears the two second fallback and does not ask again. Asking immediately caused a second tabs fetch, which `ui/reveal-tab.test.js` forbids. The two seconds delay only a node that never pushes the list.

## Evidence

One bench, 150 ms RTT, 40 iterations, errors 0, `thread_pushes` 40. `thread_message` p50 171 ms, p95 199 ms. The script reports proxy totals for the whole run, not bytes per push. `wire-contract check --base origin/main` reported no new breaks.
