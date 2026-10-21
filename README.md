# pinned

Claude Code in a small panel that stays on top of everything on macOS, including fullscreen
videos and fullscreen apps. When the agent finishes or needs your approval, the panel tells you
even while you're watching something else.

pinned hosts the official `claude` CLI in a terminal. It uses your existing Claude Code login,
and it never reads your credentials or calls the Anthropic API.

## Install

1. Install [Claude Code](https://docs.anthropic.com/en/docs/claude-code) and log in with `claude`.
2. Download the `.dmg` for your Mac from [Releases](https://github.com/FelipeGazapina/pinned/releases/latest).
   Use `aarch64` for Apple silicon and `x64` for Intel.
3. Drag `pinned.app` to Applications.

The app isn't notarized by Apple, so macOS blocks the first launch of a downloaded copy.
Run this once after you install:

```sh
xattr -dr com.apple.quarantine /Applications/pinned.app
```

## Use

- pinned starts in the project you opened most recently. Click 📁 to start `claude` in another folder.
- Drag the header to move the panel. Drag the bottom-right corner to resize it.
- Click `–` to shrink it to a pill. The pill shows what the agent is doing: working, done, or
  needs you. It flashes when the agent finishes or asks for permission. Click the pill to open
  the panel again.
- Quit from the menu-bar icon.

From a terminal you can open a specific folder:

```sh
/Applications/pinned.app/Contents/MacOS/pinned ~/path/to/project
```

## Tabs

The **＋** button in the tab bar (or ⌘T) lists your projects, most recent first: every folder
you opened with 📁, ＋, or from a terminal. Pick one (arrow keys and Enter work) to open a tab on
it, or choose **Open folder…** to add another. Esc or a click outside closes the list. Each tab
has its own Claude session and status. Each tab's dot shows its status, and the pill reports the tab that
most needs you. ✕ on a tab (or ⌘W) closes it; the app quits with the last tab. ⌘1–⌘9 switch
tabs, and ⌘← / ⌘→ move to the previous or next tab (Ctrl on other systems). 📁 changes the folder of the current tab.

Quitting pinned keeps the Claude sessions that were still going. The next launch reopens
each one in its own tab with `claude --resume`, in the same folder or worktree, so you carry
on where you stopped. A session counts as finished when you close its tab or Claude exits
(`/exit`), and pinned forgets it then. A tab where you never sent a prompt has nothing to
resume, so it isn't kept.

Continue where it stopped: if Claude was mid-turn when pinned quit, the reopened tab sends
"Continue from where you left off." by itself. The same happens when a usage limit stops a
turn: pinned reads the reset time from the session's transcript, shows `limit, continues
HH:MM` on the tab, and types the prompt once the limit lifts. If pinned was closed across
the reset, the session continues as soon as it reopens.

Drag a tab to reorder it. Drag the tab bar by its ⠿ handle or empty space to the top, bottom,
left or right side of the panel; left and right stack the tabs vertically. pinned remembers
the side you picked.

Drop files onto the panel to attach them to the current tab (images included), or paste an
image from the clipboard.

## Integrations and connectors

Click **⋯** in the header (or press ⌘,) to swap the terminal for settings. Click ⋯ again, or
pick a tab, to go back.

**Integrations** connects services. For Linear, click **Open Linear ↗**, create a personal API
key on the page that opens in your browser, and paste it. pinned checks the key with Linear
and keeps it in `~/Library/Application Support/dev.personal-harness.pinned/linear-key`,
readable only by you.

**Connectors** make the agent proactive. A connector watches Linear for issues created with a
label. For each new issue, it opens a tab in the background on the folder you chose and starts:

```sh
claude --model <model> --effort <effort> --permission-mode <mode> [--worktree=<issue>] \
  --name "<issue> <title>" "<instructions>\n\n<issue title, link and description>"
```

The default instructions ask Claude to implement the issue, record a Playwright video that
proves the fix, and open a pull request that links the video. Edit them per connector.
Each connector has its own column flow. **Move the Linear issue to the column** names
the column for each stage, and an empty field leaves the issue where it is:

- **When pinned picks it up.** pinned moves the issue before the session starts.
- **When Claude starts changing code** and **when the pull request is open.** pinned adds a
  step to the prompt that tells Claude to run `pinned stage coding` and `pinned stage review`.
  The command asks the running app to move the issue, so the Linear key never enters the
  session.

Columns are matched by name, ignoring case, within the issue's team. Several connectors can
watch the same repository with different labels and flows.

pinned checks Linear every minute and only for issues created after the connector was
saved, and it starts each issue once. Pick **Separate git worktree per issue** when several
issues may run at the same time in one repository.

### Slack

A Slack connector watches one channel for messages that @mention your bot. For each new
thread that mentions it, pinned opens a tab whose prompt holds the whole thread, and the
session answers in that thread by running `pinned slack reply "<text>"`. The bot token stays
in the app, as the Linear key does. The default instructions ask Claude to triage the thread:
analyse a bug and reply with the cause, or reply with an existing solution, without editing
code. Each Slack connector can also file a Linear issue and fix the bug, as below.

To connect Slack, create an app at api.slack.com/apps and give its bot these scopes:
`channels:history`, `groups:history`, `channels:read`, `groups:read`, `users:read` and
`chat:write`. Install the app, invite the bot to the channel with `/invite @your-bot`, and
paste the Bot User OAuth Token (`xoxb-…`) under **Integrations**. pinned keeps it in
`slack-key` next to `linear-key`.

pinned checks the channel every minute. Each thread starts one session, and a later mention in
the same thread is ignored. Mentions that arrive while pinned is closed start their session at
the next launch, and the session opens its first reply by saying it was offline. Slack rate
limits reading history, so a rate-limited check reports the error and waits for the next minute.

Turn on **Create the Linear issue and fix obvious bugs on its own** to let the session fix
bugs. This needs Linear connected and a **Team**, by name or key. A **Label** is optional. pinned
adds a decision rule to the prompt:

- If the thread is not a bug, or it already has an answer, the session replies and stops.
- If the bug is obvious and the fix is small and low-risk, the session replies that it is
  fixing it. Then it runs `pinned linear create "<title>" "<description>"`, replies with the
  issue link, fixes the bug, opens a pull request and replies with its link.
- If the bug is not obvious, or the fix is big or risky, the session replies with its analysis
  and stops.

`pinned linear create` files the issue in the connector's team with its label, and adds a link
to the Slack thread to the description. Each thread files one issue. A second call returns the
issue already filed. With the option off, pinned rejects the command. After filing, `pinned
stage coding` and `pinned stage review` move that issue through the columns you set. A thread
has no issue when pinned picks it up, so a Slack connector has no first column. The session
fixes code under the connector's **Permissions**, so pick a mode that lets it run `git` and `gh`.

## Updates

pinned checks GitHub Releases at launch and installs a newer version right away, then
restarts. It checks again every 6 hours; if a newer version exists by then, an
**Update to vX** button appears in the header (and `↑` on the pill). Click it to download,
install, and restart.

## How it works

- `src/pty.rs` runs `claude` in a pseudo-terminal. `ui/` renders it with xterm.js.
- `src/hooks.rs` passes Claude Code a `--settings` file with `UserPromptSubmit`, `Stop` and
  `Notification` hooks. Each hook runs `pinned hook <status>`, which forwards the event to the
  window over a Unix socket. Your own `~/.claude/settings.json` is not modified.
- `src/main.rs` turns the window into a non-activating `NSPanel`. macOS only lets that kind
  of window appear inside another app's fullscreen Space.

## Develop

```sh
cargo run -- ~/path/to/project    # debug build
npm run tauri build               # .app and .dmg in target/release/bundle
```

## Release

```sh
scripts/release.sh 0.2.0
```

The script bumps the version in `Cargo.toml`, tags `v0.2.0` and pushes. The `release`
workflow builds both Mac architectures, signs the update bundles, and publishes the release
with `latest.json`. Running copies then offer the update.

Update signing needs the `TAURI_SIGNING_PRIVATE_KEY` repository secret. Its public half is
`plugins.updater.pubkey` in `tauri.conf.json`. If the private key is lost, installed copies
can't verify new updates and must be reinstalled by hand.
