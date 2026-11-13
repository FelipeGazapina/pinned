# Changelog

## 0.18.2 (Unreleased)

### Added

- Ideas: the Orchestrator and your sessions can now save things you might do someday with `save_idea`, list them with `list_ideas` and delete them with `delete_idea`. Ideas are kept on your account, apart from memory, and are never fed into a prompt on their own. The Ideas page comes in a later update.
- A thread that stopped on an API error while the computer slept now shows a Needs you card saying when it slept, with a Continue button (for a Captain, also how many delegated tasks are still running). Nothing is typed for you.

### Removed

- Workflows: Generate with AI is gone. Ask an agent, such as the Orchestrator, to create a workflow from a description; older apps that still press the button get a short message saying so.

### Fixed

- Workflows: in New workflow, clicking Linear (or any Slack option) with that connector connected now continues the setup instead of opening Connectors. Since 0.17.10 every option sent you to Connectors and said "Connect Linear", whatever was connected. When the connector is missing, Connectors now says which workflow sent you there, and connecting it, or Back to the workflow, returns you to that workflow as you left it.
- A Captain no longer keeps the global Orchestrator's messages queued for hours after its turn ends. When a turn ended just as a message came in, the Captain's end-of-turn hook could give up waiting for pinned before pinned handed the message over. The turn ended without it, while pinned kept counting the Captain as busy, so every later message waited until you typed into the Captain. The hook now waits long enough for the hand-over. If a hand-over still gets lost, the queued messages start a new turn once Claude Code reports the Captain idle, about a minute later.
- Stopping a delegated agent now refuses to signal pid 0, pid 1, every process (-1), pinned itself or its own process group, so a bad pid among the agent's processes can no longer reach pinned or every process on the machine.
- Agent tabs no longer read working or starting with nothing running behind them: a session killed mid-turn now reads stopped with its reason (its time limit, stopped by you, or its exit code), sleeping and resumed tabs read idle, a start that never reports fails after 10 min, and the Orchestrator's `projects()` count skips workers with no live process.
- A worker box whose link to the hub drops for a moment keeps its task. Before, a box running for more than 5 minutes that lost its link was deleted at once as "the box did not connect within 5 min". Now its task waits for it to come back, and a box that stays offline for 10 minutes ends with that reason.
- Upstash Box: removing a box machine, disconnecting Upstash or ending a worker now deletes its box with only the Upstash key stored, and one box that will not delete no longer stops the sweep from deleting the others. A box that reaches one of its limits (time, idle, budget) is now deleted too when only the key is stored.
- A project's model ceiling (Tier) no longer lets through a Claude model id that this machine's `claude` does not list, nor an empty model name, because the words of an id are no proof of its cost. A remote worker's model is checked against the Claude list of the machine that runs it once that machine is on 0.18.2; until then the sending machine refuses a Claude id it does not list.

## 0.18.1 (2026-11-12)

### Fixed

- Finished workers no longer look active. Under a Captain in the sidebar, the workers whose turn ended now fold into a "Finished" group that opens with a click, and the workers still running, waiting on you or failed stay where they were. A Captain's Workers card stays in the turn that started its workers, and no longer follows the conversation to its end once every worker is done, and when it lists only the workers at work its header still counts every worker. The global Orchestrator's `projects()` count no longer includes workers that already finished, and the Projects page counts workers the same way.
- Slack: a mention in a thread that a workflow already follows now files a ticket when it asks for one, and a second request in the same thread answers with the same ticket. Already on main (#193).
- Dictation on macOS: clicking Allow no longer ends in "Microphone access denied". The app now carries the microphone entitlement that macOS requires before it will ask. While Microphone or Speech Recognition is off, every click on the mic says which one and opens its System Settings pane; in a browser it says how to unblock the microphone for the site. macOS may ask again for Microphone and Speech Recognition after an update, because the app is not yet signed with an Apple Developer ID.
- Settings: an action that fails now says why in a note at the bottom of the window. Before, a failed click, Enter in a token field or ⌘S showed nothing, so a refused Disconnect looked like a click that did nothing.
- Windows desktop app: MCP server values (environment variables and headers) could not be saved, and the Upstash Box Claude token was kept in a hidden NTFS stream that copies, zips and backups left out. Both are now stored under an escaped file name, and a token kept the old way is still read.

### Added

- Linux desktop beta: an AppImage and a .deb for x86_64 and arm64, next to the .dmg and the Windows setup .exe, with updater entries in `latest.json`. The AppImage needs FUSE, or run it with `APPIMAGE_EXTRACT_AND_RUN=1`. Dictation and browser-panel screenshots need macOS and are not available on Linux. Not tested yet on Linux: the tray icon on a real desktop, including GNOME, Wayland (it runs through XWayland), notifications, keep awake, and the updater end to end. Beta testers, please report problems with those. The `pinned-server-*-linux-musl` files on the same release are the headless node, not the desktop app.
- Upstash Box: a default box size for each account and each project, set under Delegation limits on the Way of working page. Already on main (#195).
- Slack: a triage session can set the ticket's title and summary before the workflow files it, with `pinned run set` and `{{vars.*}}` in the workflow's ticket fields. Already on main (#196).
- Usage page: an Upstash Box section with what Upstash reports for this month's boxes, pinned's own count of box hours, and the boxes running now. Already on main (#199).
- Sessions that run in an Upstash Box show the Upstash mark beside their name: in the sidebar (threads, Captains and workers), the Workers card, the phone Workers sheet and Needs you. The mark comes from the machine's Upstash Box field, not its name, and reads "Running in an Upstash Box" to screen readers.
- Upstash Box: a box pinned opens is named after its project and role, e.g. `pinned-worker-7fc0a1`, in the Upstash dashboard. Boxes already running keep their old `pinned-<id>` names until they end.
- Slack: connect several bots, one per workspace or Slack app. The Slack card lists them by workspace and shows which workflows use each. A workflow picks its bot in the trigger, next to the channel. Its replies, tickets and the workflows it calls use that bot, and a thread stays with the bot that first answered it. An existing Slack connection becomes the first bot, the default, with nothing to redo. Video: [connecting a second bot](https://github.com/FelipeGazapina/pinned/releases/download/v0.18.1/multi-slack-ui.webm).

### Changed

- Opening a long thread, on desktop and phone, now shows its newest 300 messages first and reads older ones as you scroll up. On a 2,000-turn thread the last turn showed in 0.65 s instead of 2.8 s. A machine still on 0.18.0 sends the whole thread as before, so update it too.
- Disconnecting Slack is now refused while any workflow uses that bot, paused workflows included. This also applies with a single bot and from an older app. Move those workflows to another bot, or delete them, then disconnect. The message names the workflows.
- On a 0.18.0 desktop app, Disconnect for Slack appears to do nothing while a workflow uses the bot. Update the app, or move or delete those workflows first.

### Rolling back to 0.18.0

- Slack: once a hub, or a desktop app used without signing in, is back on 0.18.0 and saves its settings, only the first Slack bot connected is kept. Workflows that used another bot then run with that first bot. After updating again, connect the other bots and pick them in those workflows again.
- If you roll the desktop app back to 0.18.0, enter again any MCP server value or Upstash Box Claude token you changed while on 0.18.1. 0.18.1 saves those keys under a new file name, and 0.18.0 doesn't find that file. Keys 0.18.1 only read still work after a rollback. Slack, Linear, GitHub, Vercel and Upstash keys keep their names.
