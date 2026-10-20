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

- pinned starts in the last folder you opened. Click 📁 to start `claude` in another folder.
- Drag the header to move the panel. Drag the bottom-right corner to resize it.
- Click `–` to shrink it to a pill. The pill shows what the agent is doing: working, done, or
  needs you. It flashes when the agent finishes or asks for permission. Click the pill to open
  the panel again.
- Quit from the menu-bar icon.

From a terminal you can open a specific folder:

```sh
/Applications/pinned.app/Contents/MacOS/pinned ~/path/to/project
```

## Several pinned panels

The **＋** button in the header opens another pinned panel next to this one, on the same
folder, with its own Claude session and status. **✕** closes only that panel; the app quits
with the last one. Drop files onto a panel to attach them (images included), or paste an
image from the clipboard.

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
