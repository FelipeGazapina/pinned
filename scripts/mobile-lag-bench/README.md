# Mobile lag bench

Measures how the web app responds on a phone while a busy Orchestrator streams frames at it.

`run.js` serves a `ui/` folder and opens it in headless Chrome at 390x844 with touch and an iPhone user agent, under CDP CPU throttling. A fake hub (`hub.js`) answers the page over a real WebSocket on the same port. While the hub streams frames, the run does what a person on the phone does:

1. Opens the Orchestrator tab, types a 57-character message into the composer at 120 ms per key, and scrolls the chat.
2. Opens the thread list, opens a worker's thread, and scrolls it.
3. Goes back to the worker's Captain, back to the list, opens another worker, and opens Needs you.

## Workload

The hub has one machine with the global orchestrator, 8 Captains (one per project, each with two plans), 30 workers and 41 finished threads: 80 tabs, so a `tabs` frame is about 114 KB.

The rates come from the production hub's census lines (`pinned-server: census ...`, every 5 minutes) for 2026-11-18. The busiest intervals had 2 clients. The `busy` load is that interval's traffic per client:

| frame | busiest interval, all clients | `busy`, per second | `peak`, per second |
|---|---|---|---|
| `event` `tabs` | 201 per 5 min, 137 KB each | 0.35 | 0.67 |
| `event` `output` | 73,477 per 5 min, 496 B each | 120 | 245 |
| `event` `usage` | 1,318 per 5 min | 2.2 | 4.4 |
| `event` `agent` | 48 per 5 min | 0.1 | 0.16 |
| transcript lines (a `thread` nudge when the page watches the tab) | 565 nudges per 5 min | 2.5 | 5 |

The hub sends `output` only for tabs the page attached and a `thread` nudge only for tabs it watches, as the real hub does. The page's own calls (`thread` reads on the 1200 ms poll and on nudges, `workspace`, and so on) are answered from the generated transcripts, so the reply traffic comes from the UI's behaviour. `quiet` sends nothing and marks every agent finished: the "fine when nothing runs" control.

The tab and transcript text is generated. Its sizes match the census, its words do not.

## Run

The repository has no Playwright. Point `PLAYWRIGHT` at an installed one (1.58 or newer). The browser is the installed Google Chrome (`channel: "chrome"`), or `CHROME_BIN`.

```
PLAYWRIGHT=<dir>/node_modules/playwright node scripts/mobile-lag-bench/run.js --out <dir> --throttle 4,6 --runs 3
```

| option | meaning |
|---|---|
| `--ui <dir>` | The `ui/` folder to serve. Default: this checkout's. To compare with another revision, `git archive <rev> ui \| tar -x -C <dir>` and pass `<dir>/ui`. |
| `--load busy\|peak\|quiet` | The frame rates above. Default `busy`. |
| `--throttle 4,6` | CDP CPU throttling rates to run, each once per run. |
| `--runs 3` | Runs per throttle. |
| `--port 55140` | Port of the page and the fake hub. |
| `--profile` | Writes a V8 CPU profile of the measured window and its top functions (`*.profile-top.json`). Profiling slows the page: do not report numbers from these runs. |
| `--trace` | Writes a Chrome trace with the stacks that forced style and layout. Read it with `node scripts/mobile-lag-bench/trace-top.js <trace.json>`. Not for reported numbers either. |
| `--count fnA,fnB` | Counts calls and inclusive time of these global functions. Not for reported numbers. |
| `--shots` | Screenshots of the Orchestrator, list, thread and Needs you screens of the first run. |

Each run writes `<load>-x<throttle>-run<n>.json`, and the command writes `summary-<load>.json` with the median, minimum and maximum of each metric per throttle.

## Metrics

- `mainThread`: CDP `Performance.getMetrics` deltas over the measured window. `taskMs` is all the renderer main thread's work, and `busyPct` is that over the wall time. `scriptMs`, `layoutMs` and `styleMs` split it.
- `longTasks`: tasks over 50 ms, from a `longtask` observer.
- `keystrokes`: Event Timing durations per key press (keydown to the next paint), and the input delay of keydowns.
- `taps` and `inpMs`: Event Timing durations per interaction; `inpMs` is the worst interaction, which is INP with fewer than 50 interactions.
- `scroll`: animation frames during the touch scroll gestures. A gap of n frame times counts n - 1 dropped frames.
- `received`: the frames and bytes the hub sent per second, by kind.
- `sidebarAnimations`: CSS animations running in the sidebar while the Orchestrator screen hides it (`hidden`) and while the thread list shows it (`shown`).

Compare two `ui/` folders by alternating them, one run each per round, since the machine's load drifts between runs.
