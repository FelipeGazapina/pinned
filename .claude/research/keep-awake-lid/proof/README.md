# Proof: a turn cut by a sleep (PR 2b)

Recorded on a throwaway local hub (`scripts/web-sweep/stack.sh`, own HOME and ports, invented data, fake claude). No production hub, no real data.

- `cut-by-sleep-desktop.webm` (1280 px) and `cut-by-sleep-390.webm` (390 px): sign in, open Needs you, the Stopped card, press Continue, the card goes and the thread has taken the prompt.
- How the flag was set, with no test-only code in the app: the node started with a `sleep.json` window already in its config dir (the file the app itself saves), and the fake claude's new `cut` prompt ended the turn with `API Error: Unable to connect to API (ECONNRESET)` through the real StopFailure hook. The node then flagged the tab by its normal rule; `sessions.json` held `cut_by_sleep`.
- The Captain line (N delegated tasks) is covered by `ui/needs-you.test.js` and `ui/redesign-model.test.js`, not shown in the videos.
