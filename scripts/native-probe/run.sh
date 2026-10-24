#!/usr/bin/env bash
# Usage: scripts/native-probe/run.sh [dir]
# Builds a debug app under a test identifier and HOME, runs probe.js inside its real window,
# and prints one line per check. Checks what a browser sweep can't: the window resizing on a
# view switch, a localhost page in the browser panel's native webview with its history, title,
# devtools, zoom and screenshot, open_url, and machine_call_slow with the app view's calls.
# The window shows on screen for about half a minute, and the system browser opens one
# localhost tab. Needs gh signed in: the slow-call check reads pull request #4.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
DIR="${1:-$(mktemp -d)}"
PORT="${PROBE_PORT:-8873}"
rm -rf "$DIR/home"
mkdir -p "$DIR/home" "$DIR/bin"

cp "$REPO/scripts/web-sweep/fake-claude" "$DIR/bin/claude"
printf '#!/bin/sh\nshift\nexec /bin/sh -c "$1"\n' >"$DIR/bin/shell"
chmod +x "$DIR/bin/claude" "$DIR/bin/shell"
rm -rf "$DIR/repo"
git clone -q --local "$REPO" "$DIR/repo"
git -C "$DIR/repo" remote set-url origin https://github.com/FelipeGazapina/pinned.git
(cd "$REPO" && TAURI_CONFIG='{"identifier":"dev.personal-harness.pinned-probe"}' cargo build -q --bin pinned)

# Pass 1 runs every check and leaves the app in the app view; pass 2 relaunches into it.
probe() {
  sed -e "s/__PORT__/$PORT/g" -e "s|__REPO__|$DIR/repo|g" -e "s/__PASS__/$1/g" -e "s/__GUI_WIDTH__/${2:-0}/g" -e "s/__GUI_HEIGHT__/${3:-0}/g" \
    "$REPO/scripts/native-probe/probe.js" >"$DIR/probe.js"
  node "$REPO/scripts/native-probe/server.js" "$PORT" >>"$DIR/reports.jsonl" &
  local server=$!
  GH_TOKEN="$(gh auth token)" HOME="$DIR/home" SHELL="$DIR/bin/shell" PATH="$DIR/bin:$PATH" PINNED_PROBE="$DIR/probe.js" \
    "$REPO/target/debug/pinned" >>"$DIR/app.log" 2>&1 &
  local app=$!
  for _ in $(seq 1 90); do
    kill -0 $server 2>/dev/null || break
    sleep 1
  done
  kill $app $server 2>/dev/null || true
  wait $app 2>/dev/null || true
}

: >"$DIR/reports.jsonl"
probe 1
gui=$(node -e 'const r = require("fs").readFileSync(process.argv[1], "utf8").split("\n").filter(Boolean).map(JSON.parse).find((r) => r.seen?.gui); console.log(r ? `${r.seen.gui.width} ${r.seen.gui.height}` : "0 0")' "$DIR/reports.jsonl")
probe 2 $gui
node -e '
  const lines = require("fs").readFileSync(process.argv[1], "utf8").trim().split("\n").filter(Boolean).map(JSON.parse);
  for (const r of lines) console.log(`${r.ok ? "PASS" : "FAIL"}  ${r.check}  ${JSON.stringify(r.seen)}`);
  const want = 16;
  if (lines.length < want) console.log(`only ${lines.length} of ${want} checks reported; see ${process.argv[2]}`);
  process.exit(lines.length === want && lines.every((r) => r.ok) ? 0 : 1);
' "$DIR/reports.jsonl" "$DIR/app.log"
