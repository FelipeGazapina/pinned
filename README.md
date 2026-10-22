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
- The header shows how much of your Claude subscription you have used, for example
  `5h 23%  7d 41%`. Each window has a thin bar: blue below 70%, amber from 70%, red from 90%.
  Hover over it to see the exact figures and when each window resets.
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

The default instructions ask Claude to implement the issue and open a pull request. Edit them
per connector.
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

pinned checks the channel every minute. A new mention in a thread that already has a session
does not start another one. Mentions that arrive
while pinned is closed start their session at the next launch, and the session opens its first
reply by saying it was offline. Slack rate limits reading history, so a rate-limited check
reports the error and waits for the next minute.

Later messages in a thread go to the session that answers it, with no need to mention the bot
again. pinned types each one into the session's tab as a prompt that starts with
`[Slack reply from <name> in the thread]`, and the session answers in the thread. Messages
from the bot and from other bots are skipped. If the thread's tab is closed, or pinned
restarted without resuming it, pinned opens a new session with the whole thread instead.
pinned follows threads that started in the last 7 days.

**When the thread reports a bug** sets how far the session goes:

- **Only analyze and reply.** The session replies with its analysis and never changes code.
  pinned rejects `pinned linear create`.
- **Analyze, then ask me in the thread before filing.** The session replies with its analysis
  and asks whether to file a Linear issue and fix the bug, then waits. When someone answers
  yes in the thread, the session files the issue and fixes the bug, as below. pinned rejects
  `pinned linear create` until someone has written in the thread after the session started.
- **Fix obvious bugs on its own, ask for the rest.** If the bug is obvious and the fix is small
  and low-risk, the session fixes it right away. Otherwise it asks, as above.

The second and third options need Linear connected and a **Team**, by name or key. A
**Label** is optional. To fix a bug, the session replies that it is fixing it and runs
`pinned linear create "<title>" "<description>"`. Then it replies with the issue link, fixes
the bug, opens a pull request and replies with its link. If the thread is not a bug, or it
already has an answer, the session replies and stops.

`pinned linear create` files the issue in the connector's team with its label, and adds a link
to the Slack thread to the description. Each thread files one issue. A second call returns the
issue already filed. After filing, `pinned stage coding` and `pinned stage review` move that
issue through the columns you set. A thread has no issue when pinned picks it up, so a Slack
connector has no first column. The session fixes code under the connector's **Permissions**,
so pick a mode that lets it run `git` and `gh`.

### Proof of work

Every connector session that can open a pull request must prove that the change works. These
are Linear connector sessions, and Slack connector sessions whose bug setting is not **Only
analyze and reply**. pinned appends a **Proof of work** block to the prompt, after your
instructions, so editing the instructions does not remove it. The block tells the session to:

1. Write a Playwright script in a temporary directory outside the repository.
2. Start the app the way the project documents, drive the flow that the change affects, and
   record a video with `recordVideo` and a small `slowMo`.
3. After the pull request is open, run
   `pinned evidence <video> --caption "<what the video shows>"`. A Slack session also puts
   the comment link in its last reply in the thread.

The proof is posted as a comment on the pull request. Nothing is committed to the repository:
the session never adds the video or the script to git.

`pinned evidence <video-file> --caption "<text>" [--pr <number>]` runs in the session's
folder with your own `gh` sign-in. It converts a `.webm` to `.mp4` when `ffmpeg` is
installed, and uploads the video as an asset of a release tagged `pinned-evidence`. If the
release does not exist yet, the command creates it as a prerelease on the default branch, and
it appears in the repository's Releases list under that name. Then the command posts a
`## Proof` comment with the caption and the asset link, on the pull request given by `--pr`
or on the current branch's pull request, and prints the comment link. Running the command
again with the same video file does not post a second comment. GitHub has no API that puts a
video inline in a comment, so the link opens or downloads the video. It does not play in the
pull request.

If a browser cannot show the change, because it is a library, a CLI or backend code, or if the
app cannot start in the session, the session records no video. Its `## Proof` comment then
says `No video: <reason>` and holds the test or command output that proves the change.

**Require a Playwright video, posted as a comment on the pull request** is on by default.
Clear it to leave the block out of a connector's sessions. A Slack connector that only
replies never opens a pull request, so the form hides the setting.

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
- The usage meter comes from the data Claude Code gives its statusline. The `--settings` file
  sets `pinned statusline` as the statusline, which forwards the 5-hour and 7-day figures to
  the window. Claude Code reports them only for Pro and Max subscriptions, and only after a
  session's first response, so the meter stays hidden until then. A window leaves the meter
  when it resets. If you set a `statusLine` in `~/.claude/settings.json`, `pinned statusline`
  runs it with the same input and prints its output, so your statusline still shows.
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
