# Why a long thread opens slowly

Every thread view (the Orchestrator's, a Captain's, an ordinary thread, the phone's) opens through
one path: `fetchThread` in `ui/gui.js` sends `thread { tab, from: 0 }`, the hub relays it to the
machine, and the machine's `Machine::thread` runs `thread::read`, which reads and parses the
transcript from its first byte, 8 MiB per call. The app asks again from each answer's `next` until it
holds the end, so it holds and ships the whole conversation before the latest turn can be shown. The
page already draws only the last 40 turns (`TURNS_SHOWN`) and a window of them, so the render is not
the slow step: the read from the start and its size on the wire are.

The app also reads every open tab this way when it loads, not just the one on screen.

## Measured (`scripts/history-bench/run.sh 2000 3`, debug build, local hub + node, headless Chromium)

Invented transcript of 2,000 turns (prompts, reasoning, Bash calls and output, a compaction, pictures,
queued prompts): 16.8 MB, 10,022 lines.

| step | before |
| --- | --- |
| reads the app makes to reach the end | 2 calls from byte 0 |
| items shipped | 12,021 |
| bytes on the wire through the hub | 9.6 MB |
| time for those reads (node read + parse + hub relay) | 2,773 ms (median of 3) |
| sign-in to the last turn on screen with the composer usable | 2,809 ms |
| tab listed to the last turn on screen | 2,274 ms |
