#!/usr/bin/env bash
# Usage: scripts/native-probe/go-thread.sh [dir]
# The check of scripts/web-sweep/go-thread.js inside the real desktop window. Builds a debug app
# under a test identifier and HOME, adds an OpenCode Go account whose gateway is
# mock-opencode-go, and runs scripts/web-sweep/go-thread-page.js once per variant with a real
# claude (REAL_CLAUDE, default ~/.local/bin/claude) on that account. Prints one line per variant.
# GO_THREAD_VARIANTS picks variants by name; PROBE_SKIP_BUILD=1 reuses the last build. The window
# shows on screen while it runs.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
DIR="${1:-$(mktemp -d)}"
PORT="${PROBE_PORT:-8873}"
GO_PORT="${PROBE_GO_PORT:-8874}"
CLAUDE="${REAL_CLAUDE:-$HOME/.local/bin/claude}"
rm -rf "$DIR/home" "$DIR/bin"
mkdir -p "$DIR/home/work" "$DIR/bin"

cp "$REPO/scripts/web-sweep/fake-claude" "$DIR/bin/fake-claude"
# pinned sets PINNED_OPENCODE_GO_KEY only on a Go session's process.
printf '#!/bin/sh\n[ -n "${PINNED_OPENCODE_GO_KEY:-}" ] && DISABLE_AUTOUPDATER=1 CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 exec "%s" "$@"\nexec "%s" "$@"\n' "$CLAUDE" "$DIR/bin/fake-claude" >"$DIR/bin/claude"
printf '#!/bin/sh\nshift\nexec /bin/sh -c "$1"\n' >"$DIR/bin/shell"
chmod +x "$DIR/bin/claude" "$DIR/bin/fake-claude" "$DIR/bin/shell"
git -C "$DIR/home/work" init -q alpha
[ -n "${PROBE_SKIP_BUILD:-}" ] || (cd "$REPO" && TAURI_CONFIG='{"identifier":"dev.personal-harness.pinned-probe"}' cargo build -q --bin pinned)

node "$REPO/scripts/web-sweep/mock-opencode-go" "$GO_PORT" >"$DIR/go.log" 2>&1 &
mock=$!
: >"$DIR/reports.jsonl"
{
  cat "$REPO/scripts/web-sweep/go-thread-page.js"
  sed -e "s/__PORT__/$PORT/g" -e "s/__GO_PORT__/$GO_PORT/g" -e "s|__FOLDER__|$DIR/home/work/alpha|g" -e "s/__VARIANTS__/${GO_THREAD_VARIANTS:-}/g" "$REPO/scripts/native-probe/go-thread.js"
} >"$DIR/probe.js"
node "$REPO/scripts/native-probe/server.js" "$PORT" >>"$DIR/reports.jsonl" &
server=$!
go="http://127.0.0.1:$GO_PORT"
HOME="$DIR/home" SHELL="$DIR/bin/shell" PATH="$DIR/bin:$PATH" PINNED_PROBE="$DIR/probe.js" \
  PINNED_OPENCODE_GO_MODELS_BASE="$go" PINNED_OPENCODE_GO_USAGE_URL="$go/zen/go/v1/usage" PINNED_OPENCODE_GO_BASE="$go/inference/go/anthropic" \
  "${CARGO_TARGET_DIR:-$REPO/target}/debug/pinned" >>"$DIR/app.log" 2>&1 &
app=$!
for _ in $(seq 1 900); do
  kill -0 $server 2>/dev/null || break
  sleep 1
done
kill $app $server $mock 2>/dev/null || true
wait $app 2>/dev/null || true

node -e '
  const fs = require("fs");
  const path = require("path");
  const lines = fs.readFileSync(process.argv[1], "utf8").split("\n").filter(Boolean).map(JSON.parse);
  const projects = path.join(process.argv[2], "home/Library/Application Support/dev.personal-harness.pinned-probe/accounts/go/projects");
  for (const r of lines) {
    const file = r.session && fs.existsSync(projects) && fs.readdirSync(projects).map((p) => path.join(projects, p, `${r.session}.jsonl`)).find((f) => fs.existsSync(f));
    for (const line of file ? fs.readFileSync(file, "utf8").split("\n").filter(Boolean) : []) {
      const entry = JSON.parse(line);
      for (const b of entry.type === "assistant" ? entry.message.content : []) {
        const name = b.type === "thinking" ? "disk.thinking" : b.name === "Bash" ? "disk.bash" : b.name === "AskUserQuestion" ? "disk.ask" : null;
        if (name && !(name in r.seen)) r.seen[name] = +((Date.parse(entry.timestamp) - r.t0) / 1000).toFixed(2);
      }
    }
    console.log(`${r.ok ? "PASS" : "FAIL"}  ${r.variant}  ${JSON.stringify(r)}`);
  }
  if (!lines.length) console.log(`no variant reported; see ${process.argv[2]}/app.log`);
  process.exit(lines.length && lines.every((r) => r.ok) ? 0 : 1);
' "$DIR/reports.jsonl" "$DIR"
