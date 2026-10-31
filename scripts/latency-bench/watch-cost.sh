#!/bin/bash
# CPU of one node process with 15 restored sessions and 300 project directories.
# The watcher starts inside `pinned-server node` whether or not a hub accepts
# the dial. HOME and PATH stay inside the scratch dir, so this never reads the
# user's Claude home or launches the installed claude.
#
#   scripts/latency-bench/watch-cost.sh
#   WATCH_OUT=.audit/watch-cpu.json scripts/latency-bench/watch-cost.sh
set -euo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
DIR=${WATCH_DIR:-${TMPDIR:-/tmp}/pinned-watch-cost}
OUT=${WATCH_OUT:-$DIR/result.json}
WINDOWS=${WATCH_WINDOWS:-5}
WINDOW_S=${WATCH_WINDOW_S:-8}
TABS=${WATCH_TABS:-15}
PROJECTS=${WATCH_PROJECTS:-300}
SERVER=${CARGO_TARGET_DIR:-$REPO/target}/release/pinned-server

case "$DIR" in
  *pinned-watch-cost*) ;;
  *) echo "refusing to use scratch dir $DIR" >&2; exit 1 ;;
esac

if [ ! -x "$SERVER" ]; then
  echo "missing $SERVER" >&2
  exit 1
fi

cleanup() {
  local status=$?
  set +e
  if [ -n "${NODE_PID:-}" ]; then kill "$NODE_PID" 2>/dev/null; wait "$NODE_PID" 2>/dev/null; fi
  set -e
  return "$status"
}
trap cleanup EXIT

rm -rf "$DIR"
mkdir -p "$DIR/bin" "$DIR/home/.claude/projects" "$DIR/config" "$DIR/work" "$(dirname "$OUT")"

cat >"$DIR/bin/claude" <<'EOF'
#!/bin/sh
if [ "$1" = "auth" ]; then
  printf '%s\n' '{"loggedIn":true,"email":"watch@example.test"}'
  exit 0
fi
exec /bin/sleep 3600
EOF
chmod +x "$DIR/bin/claude"
printf '#!/bin/sh\nprintf "\\n__PINNED_PATH__%%s__PINNED_PATH__\\n" "%s"\n' "$DIR/bin" >"$DIR/bin/shell"
chmod +x "$DIR/bin/shell"

node -e '
const fs = require("fs");
const dir = process.argv[1];
const work = process.argv[2];
const tabs = Number(process.argv[3]);
const projects = Number(process.argv[4]);
const root = dir + "/home/.claude/projects";
for (let i = 0; i < projects; i += 1) fs.mkdirSync(`${root}/p${String(i).padStart(3, "0")}`, { recursive: true });
const saved = [];
for (let i = 1; i <= tabs; i += 1) {
  const id = String(i).padStart(3, "0");
  const session = `s${id}`;
  const file = `${root}/p${id}/${session}.jsonl`;
  fs.writeFileSync(file, JSON.stringify({ type: "assistant", uuid: `u${id}`, timestamp: "2020-01-01T00:00:00Z", message: { role: "assistant", content: [{ type: "text", text: "ready" }] } }) + "\n");
  saved.push({ cwd: work, session, env: [], account: "claude", bypass: false, interrupted: false });
}
fs.writeFileSync(dir + "/config/sessions.json", JSON.stringify(saved));
fs.writeFileSync(dir + "/config/account.json", JSON.stringify({
  hub: "http://127.0.0.1:9",
  email: "watch@example.test",
  user_token: "watch-token",
  machine_id: "m-watch",
  machine_token: "watch-machine",
}));
' "$DIR" "$DIR/work" "$TABS" "$PROJECTS"

env HOME="$DIR/home" SHELL="$DIR/bin/shell" PATH="$DIR/bin:/usr/bin:/bin" \
  "$SERVER" node --config-dir "$DIR/config" >"$DIR/node.log" 2>&1 &
NODE_PID=$!

for _ in $(seq 1 50); do
  if ! kill -0 "$NODE_PID" 2>/dev/null; then
    echo "node exited" >&2
    cat "$DIR/node.log" >&2
    exit 1
  fi
  if [ "$(ps -p "$NODE_PID" -o etime= | tr -d ' ')" != "" ]; then
    sleep 1
    break
  fi
  sleep 0.1
done
sleep 2
if ! kill -0 "$NODE_PID" 2>/dev/null; then
  echo "node exited during warmup" >&2
  cat "$DIR/node.log" >&2
  exit 1
fi
kids=$(pgrep -P "$NODE_PID" | wc -l | tr -d ' ')
open=$(node -e 'const fs=require("fs"); const raw=fs.readFileSync(process.argv[1],"utf8"); const rows=JSON.parse(raw); if(!Array.isArray(rows)) process.exit(2); process.stdout.write(String(rows.length));' "$DIR/config/sessions.json")
if [ "$kids" -lt "$TABS" ] || [ "$open" -lt "$TABS" ]; then
  echo "watcher fixture is not live: children $kids sessions $open" >&2
  cat "$DIR/node.log" >&2
  exit 1
fi

node -e '
const { spawnSync } = require("child_process");
const fs = require("fs");
const pid = Number(process.argv[1]);
const windows = Number(process.argv[2]);
const seconds = Number(process.argv[3]);
const out = process.argv[4];
const tabs = Number(process.argv[5]);
const projects = Number(process.argv[6]);
function cpuSeconds() {
  const text = spawnSync("ps", ["-p", String(pid), "-o", "time="], { encoding: "utf8" }).stdout.trim();
  const parts = text.split(":").map(Number);
  if (parts.some((n) => !Number.isFinite(n))) throw new Error(`bad cpu time ${text}`);
  if (parts.length === 2) return parts[0] * 60 + parts[1];
  if (parts.length === 3) return parts[0] * 3600 + parts[1] * 60 + parts[2];
  throw new Error(`bad cpu time ${text}`);
}
function load() {
  return spawnSync("sysctl", ["-n", "vm.loadavg"], { encoding: "utf8" }).stdout.trim();
}
const samples = [];
for (let i = 0; i < windows; i += 1) {
  const loadBefore = load();
  const start = cpuSeconds();
  spawnSync("sleep", [String(seconds)]);
  const end = cpuSeconds();
  const pct = ((end - start) / seconds) * 100;
  samples.push({ pct: Math.round(pct * 10) / 10, cpu_s: Math.round((end - start) * 1000) / 1000, load: loadBefore });
  process.stderr.write(`window ${i + 1} ${pct.toFixed(1)}%\n`);
}
const sorted = samples.map((s) => s.pct).sort((a, b) => a - b);
const mid = sorted[Math.floor(sorted.length / 2)];
const body = {
  pid,
  tabs,
  projects,
  windows,
  window_s: seconds,
  median_pct: mid,
  min_pct: sorted[0],
  max_pct: sorted[sorted.length - 1],
  samples,
};
fs.writeFileSync(out, JSON.stringify(body, null, 2) + "\n");
console.log(JSON.stringify(body));
' "$NODE_PID" "$WINDOWS" "$WINDOW_S" "$OUT" "$TABS" "$PROJECTS"

echo "result $OUT"
if [ -n "${NODE_PID:-}" ] && kill -0 "$NODE_PID" 2>/dev/null; then
  sample "$NODE_PID" 2 -file "$DIR/sample.txt" >/dev/null 2>&1 || true
  if [ -f "$DIR/sample.txt" ]; then
    cp "$DIR/sample.txt" "${OUT%.json}-sample.txt"
  fi
fi
