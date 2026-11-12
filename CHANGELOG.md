# Changelog

## 0.18.1 (Unreleased)

### Fixed

- Finished workers no longer look active. Under a Captain in the sidebar, the workers whose turn ended now fold into a "Finished" group that opens with a click, and the workers still running, waiting on you or failed stay where they were. A Captain's Workers card stays in the turn that started its workers, and no longer follows the conversation to its end once every worker is done, and when it lists only the workers at work its header still counts every worker. The global Orchestrator's `projects()` count no longer includes workers that already finished, and the Projects page counts workers the same way.
- Slack: a mention in a thread that a workflow already follows now files a ticket when it asks for one, and a second request in the same thread answers with the same ticket. Already on main (#193).
- Dictation on macOS: clicking Allow no longer ends in "Microphone access denied". The app now carries the microphone entitlement that macOS requires before it will ask. While Microphone or Speech Recognition is off, every click on the mic says which one and opens its System Settings pane; in a browser it says how to unblock the microphone for the site. macOS may ask again for Microphone and Speech Recognition after an update, because the app is not yet signed with an Apple Developer ID.

### Added

- Linux desktop beta: an AppImage and a .deb for x86_64 and arm64, next to the .dmg and the Windows setup .exe, with updater entries in `latest.json`. The AppImage needs FUSE, or run it with `APPIMAGE_EXTRACT_AND_RUN=1`. Dictation and browser-panel screenshots need macOS and are not available on Linux. Not tested yet on Linux: the tray icon on a real desktop, including GNOME, Wayland (it runs through XWayland), notifications, keep awake, and the updater end to end. Beta testers, please report problems with those. The `pinned-server-*-linux-musl` files on the same release are the headless node, not the desktop app.
- Upstash Box: a default box size for each account and each project, set under Delegation limits on the Way of working page. Already on main (#195).
- Slack: a triage session can set the ticket's title and summary before the workflow files it, with `pinned run set` and `{{vars.*}}` in the workflow's ticket fields. Already on main (#196).
- Usage page: an Upstash Box section with what Upstash reports for this month's boxes, pinned's own count of box hours, and the boxes running now. Already on main (#199).
- Sessions that run in an Upstash Box show the Upstash mark beside their name: in the sidebar (threads, Captains and workers), the Workers card, the phone Workers sheet and Needs you. The mark comes from the machine's Upstash Box field, not its name, and reads "Running in an Upstash Box" to screen readers.
- Upstash Box: a box pinned opens is named after its project and role, e.g. `pinned-worker-7fc0a1`, in the Upstash dashboard. Boxes already running keep their old `pinned-<id>` names until they end.
