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
most needs you. ✕ on a tab (or ⌘W) closes it; signed out, the app quits with the last tab. ⌘1–⌘9 switch
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

Drop files onto the panel to attach them to the current tab (images included), paste an
image from the clipboard, or click **📎** to pick files. A tab on another machine gets each
file sent there first, up to 25 MB, into a private `uploads` folder in that machine's config
folder. Uploads older than a day are deleted.

## Accounts

An account is one login of an agent CLI: a Claude Code subscription or a Codex login. Each
project picks the account that runs its sessions, and pays for them, so a work project and a
personal one can use different subscriptions on the same Mac.

Settings → **Accounts** lists them, with the email, organization and plan each CLI reports.
Two are built in and can't be removed: **Claude Code**, the login in `~/.claude`, and
**Codex**, the login in `~/.codex`. Without changing anything, every project runs under the
built-in Claude Code account, as before.

To add one, pick the provider, type a name such as `Work` and click **Add account**. Then click
**Sign in**: pinned opens a tab running the CLI's own login (`claude auth login` or `codex
login`) for that account. Finish it in the browser page it opens and close the tab. **Sign in
again** switches the login of an account that already has one.

Each added account keeps its credentials in a home of its own, which pinned passes to the CLI
as `CLAUDE_CONFIG_DIR` or `CODEX_HOME`: on a Mac,
`~/Library/Application Support/dev.personal-harness.pinned/accounts/<id>`, and on a server,
`accounts/<id>` in the node's config directory. pinned never reads the credentials. Removing
an account leaves its home in place, so adding it again finds it signed in.

Pick a project's account from the menu on its row under Settings → **Projects**. Tabs opened on
the project from then on run under that account, and a resumed session reopens under the
account it started with. A tab that runs under an account other than the built-in Claude Code
one shows that account's name in the tab bar.

A connector runs under its project's account unless its **Run a session** step picks one
under **Account**. Only Claude Code accounts can run a connector. A machine where that account
is missing or signed out refuses the run, so it goes to the next online machine where it is
signed in.

The usage meter shows the active tab's account. Each Claude Code account has its own limits.

Codex tabs run the `codex` CLI without pinned's hooks, so they get no status dot, no usage
meter and no resume after a restart.

## Machines

Sign in under **Settings → Account** to use pinned from more than one machine. The hub
(`https://pinned-hub-production.up.railway.app` unless you change it) holds your account. Enter your
email and a password, then click **Sign in**, or **Create account** the first time. This Mac
joins the account under its computer name.

While you are signed in:

- The tab bar shows the tabs of every machine that is online. A tab on another machine
  carries a badge with that machine's initials, and its tooltip names the machine. Typing,
  resizing, closing and restarting (Enter after claude exits) all happen on the tab's machine.
  A tab that opens or closes on one device does the same on every other device.
- **＋** shows a row of your machines above the projects, with this Mac first. Offline machines
  are greyed out. Pick a machine, or press ← and →, to list its projects. On another machine,
  **Browse folders…** replaces **Open folder…**: type a path, or pick folders to step into
  them. If that folder doesn't exist there, the list says so. 📁 on another machine's tab opens the same list, to move the tab to another folder.
- The account holds your workflows, Linear and Slack keys, and the hub runs the workflows
  even when this Mac sleeps. The first time you sign in, pinned moves this Mac's workflows
  and keys into the account, if the account has none yet. A workflow's **Run on** field picks
  the machine its sessions start on; when that machine is offline, another online machine
  takes the run and finds or clones the repository there.
- Closing the last tab leaves pinned running, so your other devices can still open sessions
  on this Mac.

**Settings → Machines** lists every machine in the account. A green dot means online, and an
offline machine shows when it was last seen. Click **Rename** to rename a machine. Click
**Remove** to take a machine out of the account; a second row asks you to confirm.
**Sign out** under Account removes this Mac from the account, and pinned runs its own
workflows again.

### Add a server

A Linux server (x86_64 or arm64) can run sessions too. Under
**Settings → Machines**, click **Get a pairing command** and run the command it shows on the
server:

```sh
curl -fsSL https://pinned-hub-production.up.railway.app/install.sh | sh -s -- K7Q2M9XD
```

The code works once, for 15 minutes. The script installs `pinned-server`, pairs it with your
account, and starts it as a systemd service, so it comes back after a reboot. The server needs
`claude` installed and signed in, the same as this Mac.

### Web app

Open the hub's address in any browser, a phone's included, and sign in with the same email
and password. You get the same tabs and settings as in the app, without the panel controls.
On a phone the tabs sit in one scrolling row on top; hold a tab to drag it to another place.
A row of keys above the terminal gives you Esc, Tab, ⇧Tab, ↑, ↓, Ctrl-C and Enter. **Paste**
types the clipboard's text, and **Select** shows the terminal's text so you can select and
copy it; tap **Select** again to go back. The terminal shrinks to stay above the on-screen
keyboard.

On Android you can install the web app, so it opens from the home screen in its own window
like a native app. Open the hub's address in Chrome, tap the ⋮ menu, then tap **Install app**
(older versions say **Add to Home screen**). Chrome offers this only over https, which is why
the hub runs on Railway at its railway.app address. The installed app opens on the same
sign-in form, and no session can start until you sign in. Without a network it still opens,
but it cannot reach your machines until the network comes back.

**📎** attaches files the same way the app does. On a phone it offers the camera, your photos
and your files. In a desktop browser you can also drop files on the terminal or paste an image.

To hear when a session finishes or needs you, click **Enable** under **Settings → Account →
Notifications on this device**. A notification shows when that happens while the page is in
the background or shows another session, and tapping it opens that session's tab. The page title and the
installed app's icon count the sessions you haven't looked at yet. Notifications arrive only
while the web app is open or in the background. With the app fully closed they would need
Web Push, which pinned does not have yet.

## Connectors and workflows

Click **⋯** in the header (or press ⌘,) to swap the terminal for settings. Click ⋯ again, or
pick a tab, to go back.

**Connectors** connects services. For Linear, click **Open Linear ↗**, create a personal API
key on the page that opens in your browser, and paste it. pinned checks the key with Linear
and keeps it in `~/Library/Application Support/dev.personal-harness.pinned/linear-key`,
readable only by you.

**Workflows** make the agent proactive. A workflow watches Linear for issues created with a
label. For each new issue, it opens a tab in the background on the folder you chose and starts:

```sh
claude --model <model> --effort <effort> --permission-mode <mode> [--worktree=<issue>] \
  --name "<issue> <title>" "<instructions>\n\n<issue title, link and description>"
```

The default instructions ask Claude to implement the issue and open a pull request. Edit them
per workflow.

The Workflows page lists each workflow with its steps and a sentence that says what it does.
The switch on a workflow turns it on or off right away. Click a workflow to open its flow:
the trigger at the top, then each step it runs, joined by the event that leads to it, as in
n8n. Click a step to set it up, and click it again to fold it. A dashed **＋** step is optional
and off: click it to add it, and **Remove step** turns it off again. A **!** marks a step that
is missing something, and saving opens that step. ⌘S saves. **＋ New workflow** asks what
starts the agent, Linear or Slack, and opens a flow filled with defaults.

Each workflow has its own column flow. Each **Move the issue** step names the column for a
stage, and a stage without the step leaves the issue where it is:

- **When pinned picks it up.** pinned moves the issue before the session starts.
- **When Claude starts changing code** and **when the pull request is open.** pinned adds a
  step to the prompt that tells Claude to run `pinned stage coding` and `pinned stage review`.
  The command asks the running app to move the issue, so the Linear key never enters the
  session.

Columns are matched by name, ignoring case, within the issue's team. Several workflows can
watch the same repository with different labels and flows.

pinned checks Linear every minute and only for issues created after the workflow was
saved, and it starts each issue once. Pick **Separate git worktree per issue** when several
issues may run at the same time in one repository.

### Slack

A Slack workflow watches one channel for messages that @mention your bot. For each new
thread that mentions it, pinned opens a tab whose prompt holds the whole thread, and the
session answers in that thread by running `pinned slack reply "<text>"`. The bot token stays
in the app, as the Linear key does. The default instructions ask Claude to triage the thread:
analyse a bug and reply with the cause, or reply with an existing solution, without editing
code. Each Slack workflow can also file a Linear issue and fix the bug, as below.

To connect Slack, create an app at api.slack.com/apps and give its bot these scopes:
`channels:history`, `groups:history`, `channels:read`, `groups:read`, `users:read`,
`chat:write` and `reactions:write`. Install the app, invite the bot to the channel with
`/invite @your-bot`, and paste the Bot User OAuth Token (`xoxb-…`) under **Connectors**.
pinned keeps it in `slack-key` next to `linear-key`.

pinned checks the channel every minute. A new mention in a thread that already has a session
does not start another one. Mentions that arrive
while pinned is closed start their session at the next launch, and the session opens its first
reply by saying it was offline. Slack rate limits reading history, so a rate-limited check
reports the error and waits for the next minute.

Later messages in a thread go to the session that answers it only when one of them mentions
the bot, so people can talk in the thread without the session answering each message. When a
message mentions the bot, pinned types it into the session's tab with every message written
since the last one the session got, each as a prompt that starts with
`[Slack reply from <name> in the thread]`, and the session answers in the thread. Messages
from the bot and from other bots are skipped. If the thread's tab is closed, or pinned
restarted without resuming it, pinned opens a new session with the whole thread instead.
pinned follows threads that started in the last 7 days.

When a session starts on a message that mentions the bot, the bot reacts to it with
:robot_face:, and it does the same to each later mention the session receives, so people can
tell the mention reached a session.

The **If it reports a bug** step sets how far the session goes:

- **Only analyze and reply.** The session replies with its analysis and never changes code.
  pinned rejects `pinned linear create`.
- **Analyze, then ask me in the thread before filing.** The session replies with its analysis
  and asks whether to file a Linear issue and fix the bug, then waits. When someone answers
  yes in the thread and mentions the bot, the session files the issue and fixes the bug, as
  below. pinned rejects `pinned linear create` until someone has mentioned the bot in the
  thread after the session started.
- **Fix obvious bugs on its own, ask for the rest.** If the bug is obvious and the fix is small
  and low-risk, the session fixes it right away. Otherwise it asks, as above.

The second and third options need Linear connected and a **Team**, by name or key. A
**Label** is optional. To fix a bug, the session replies that it is fixing it and runs
`pinned linear create "<title>" "<description>"`. Then it replies with the issue link, fixes
the bug, opens a pull request and replies with its link. If the thread is not a bug, or it
already has an answer, the session replies and stops.

`pinned linear create` files the issue in the workflow's team with its label, and adds a link
to the Slack thread to the description. pinned files on your behalf, so the issue is assigned to
the owner of the connected Linear account. If the thread names who should own the task, the
session adds `--assignee "<name or email>"`, and pinned matches it against the workspace's
users by name, display name or email, ignoring case. Each thread files one issue. A second call returns the
issue already filed. After filing, `pinned stage coding` and `pinned stage review` move that
issue through the columns you set. A thread has no issue when pinned picks it up, so a Slack
workflow has no first column. The session fixes code under the workflow's **Permissions**,
so pick a mode that lets it run `git` and `gh`.

### One session per issue

An issue gets one session across all workflows. A Linear workflow does not start a session
for an issue that a Slack thread's session filed or took, even when the issue carries the
workflow's label, and two Linear workflows never both start the same issue.

When a thread asks the session to work on an existing issue, the session runs
`pinned linear take <identifier>`, for example `pinned linear take ENG-123`, before anything
else. If another session already works on the issue, the command says which workflow's
session has it, and the session replies in the thread that the issue is already being worked
on and stops. Otherwise the thread takes the issue: the session replies that it is taking it
and fixes it as above, `pinned stage` moves that issue, and the Linear workflow skips it.
The thread asked for the issue, so taking one does not wait for a reply under **Analyze, then
ask me in the thread before filing**. A workflow set to **Only analyze and reply** never takes
an issue.

### Proof of work

Every workflow session that can open a pull request must prove that the change works. These
are Linear workflow sessions, and Slack workflow sessions whose bug setting is not **Only
analyze and reply**. pinned appends a **Proof of work** block to the prompt, after your
instructions, so editing the instructions does not remove it. The block tells the session to:

1. Write a Playwright script in a temporary directory outside the repository.
2. Start the app the way the project documents, then record the flow that the change affects
   with the `pinned:tutorial-video` skill, which pinned loads into every session it starts
   (`plugin/`, passed with `--plugin-dir`). The video reaches the
   feature by clicking the sidebar, nav bar and tabs with a visible cursor, never by pasting a
   URL, and the helper fails the run if the page changes without a click.
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

The **Prove it works** step is on by default. Remove it to leave the block out of a
workflow's sessions. A Slack workflow that only replies never opens a pull request, so its
flow has no such step.

## Updates

pinned checks GitHub Releases at launch and installs a newer version right away, then
restarts. It checks again every 6 hours; if a newer version exists by then, an
**Update to vX** button appears in the header (and `↑` on the pill). Click it to download,
install, and restart.

## How it works

- The code is a cargo workspace. `crates/core` (`pinned-core`) holds everything that doesn't
  need a window, and `src/` is the Tauri app built on it.
- `crates/core/src/machine.rs` runs each tab's `claude` in a pseudo-terminal
  (`crates/core/src/pty.rs`) and keeps the last 256 KiB of its output for whoever attaches.
  `ui/` renders it with xterm.js.
- `crates/core/src/hooks.rs` passes Claude Code a `--settings` file with `UserPromptSubmit`, `Stop` and
  `Notification` hooks. Each hook runs `pinned hook <status>`, which forwards the event to the
  window over a Unix socket. Your own `~/.claude/settings.json` is not modified.
- The usage meter comes from the data Claude Code gives its statusline. The `--settings` file
  sets `pinned statusline` as the statusline, which forwards the 5-hour and 7-day figures to
  the window. Claude Code reports them only for Pro and Max subscriptions, and only after a
  session's first response, so the meter stays hidden until then. A window leaves the meter
  when it resets. If you set a `statusLine` in `~/.claude/settings.json`, `pinned statusline`
  runs it with the same input and prints its output, so your statusline still shows.
- Signed in, `crates/core/src/link.rs` keeps one outbound WebSocket from this Mac to the hub,
  which routes calls and terminal output between machines and clients.
  `crates/server` (`pinned-server`) is the hub and the headless node, and the hub serves
  `ui/` as the web app. `crates/core/src/wire.rs` defines every message between them.
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
