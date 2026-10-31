#!/bin/bash
# Phone-to-machine latency bench. Throwaway Postgres, a local hub, and a node
# whose claude is scripts/latency-bench/fake-claude. A TCP proxy on each side
# of the hub holds bytes for half of RTT_MS (default 150). Prints p50/p95 and
# writes the JSON next to the scratch dir.
#
#   scripts/latency-bench/run.sh
#   RTT_MS=0 scripts/latency-bench/run.sh
#   ITERATIONS=40 RTT_MS=150 scripts/latency-bench/run.sh
#
# LATENCY_DIR, LATENCY_PG_PORT (5503), LATENCY_HUB_PORT (8791),
# LATENCY_CLIENT_PORT (8792) and LATENCY_NODE_PORT (8793) override the scratch
# location and ports. Nothing here talks to a hub that is not on 127.0.0.1.
set -euo pipefail
set +m

REPO=$(cd "$(dirname "$0")/../.." && pwd)
DIR=${LATENCY_DIR:-${TMPDIR:-/tmp}/pinned-latency-bench}
PG_PORT=${LATENCY_PG_PORT:-5503}
HUB_PORT=${LATENCY_HUB_PORT:-8791}
CLIENT_PORT=${LATENCY_CLIENT_PORT:-8792}
NODE_PORT=${LATENCY_NODE_PORT:-8793}
RTT_MS=${RTT_MS:-150}
ITERATIONS=${ITERATIONS:-40}
WARMUP=${WARMUP:-2}
HEAVY=${HEAVY:-}
EMAIL=latency@example.test
PASSWORD="latency password 1"
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
export CARGO_BUILD_JOBS=4
SERVER=${CARGO_TARGET_DIR:-$REPO/target}/release/pinned-server

case "$DIR" in
  *pinned-latency-bench*) ;;
  *) echo "refusing to use scratch dir $DIR" >&2; exit 1 ;;
esac
case "$PG_PORT$HUB_PORT$CLIENT_PORT$NODE_PORT" in
  *[!0-9]*) echo "ports must be numeric" >&2; exit 1 ;;
esac

cleanup() {
  local status=$?
  set +e
  if [ -n "${BENCH_PID:-}" ]; then kill "$BENCH_PID" 2>/dev/null; fi
  if [ -f "$DIR/node.pid" ]; then kill "$(cat "$DIR/node.pid")" 2>/dev/null; fi
  if [ -f "$DIR/client-proxy.pid" ]; then kill "$(cat "$DIR/client-proxy.pid")" 2>/dev/null; fi
  if [ -f "$DIR/node-proxy.pid" ]; then kill "$(cat "$DIR/node-proxy.pid")" 2>/dev/null; fi
  if [ -f "$DIR/hub.pid" ]; then kill "$(cat "$DIR/hub.pid")" 2>/dev/null; fi
  if [ -d "$DIR/pg" ]; then pg_ctl -D "$DIR/pg" -m fast stop >/dev/null 2>&1; fi
  wait 2>/dev/null
  set -e
  return "$status"
}

wait_port() {
  local port=$1
  local i
  for i in $(seq 1 50); do
    if (echo >/dev/tcp/127.0.0.1/"$port") >/dev/null 2>&1; then return 0; fi
    sleep 0.05
  done
  echo "port $port did not open" >&2
  exit 1
}
trap cleanup EXIT

if [ -d "$DIR" ]; then
  cleanup
  rm -rf "$DIR"
fi
mkdir -p "$DIR/bin" "$DIR/home/work" "$DIR/downloads" "$DIR/state"

cargo build -p pinned-server --release --manifest-path "$REPO/Cargo.toml"

export LC_ALL=C
initdb -U pinned --auth=trust -D "$DIR/pg" >/dev/null
pg_ctl -D "$DIR/pg" -l "$DIR/pg.log" -o "-p $PG_PORT -c listen_addresses=127.0.0.1 -c unix_socket_directories=" start >/dev/null
for _ in $(seq 1 50); do
  psql -h 127.0.0.1 -p "$PG_PORT" -U pinned -d postgres -c 'select 1' >/dev/null 2>&1 && break
  sleep 0.2
done
psql -h 127.0.0.1 -p "$PG_PORT" -U pinned -d postgres -qc "create database pinned"

"$SERVER" hub --db "postgres://pinned@127.0.0.1:$PG_PORT/pinned" --listen "127.0.0.1:$HUB_PORT" \
  --ui "$REPO/ui" --downloads "$DIR/downloads" --key-file "$DIR/hub.key" \
  --public-url "http://127.0.0.1:$HUB_PORT" --signup open >"$DIR/hub.log" 2>&1 &
echo $! >"$DIR/hub.pid"
for _ in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:$HUB_PORT/" >/dev/null 2>&1 && break
  sleep 0.2
done
curl -fsS "http://127.0.0.1:$HUB_PORT/" >/dev/null

node "$REPO/scripts/latency-bench/proxy.js" --listen "127.0.0.1:$CLIENT_PORT" --upstream "127.0.0.1:$HUB_PORT" \
  --rtt-ms "$RTT_MS" --stats "$DIR/client-proxy.stats" >"$DIR/client-proxy.log" 2>&1 &
echo $! >"$DIR/client-proxy.pid"
node "$REPO/scripts/latency-bench/proxy.js" --listen "127.0.0.1:$NODE_PORT" --upstream "127.0.0.1:$HUB_PORT" \
  --rtt-ms "$RTT_MS" --stats "$DIR/node-proxy.stats" >"$DIR/node-proxy.log" 2>&1 &
echo $! >"$DIR/node-proxy.pid"
wait_port "$CLIENT_PORT"
wait_port "$NODE_PORT"

curl -fsS "http://127.0.0.1:$HUB_PORT/api/signup" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" >/dev/null

cp "$REPO/scripts/latency-bench/fake-claude" "$DIR/bin/claude"
chmod +x "$DIR/bin/claude"
LATENCY_PATH="$DIR/bin:/opt/homebrew/bin:/usr/bin:/bin"
printf '#!/bin/sh\nprintf "\\n__PINNED_PATH__%%s__PINNED_PATH__\\n" "%s"\n' "$LATENCY_PATH" >"$DIR/bin/shell"
chmod +x "$DIR/bin/shell"

env HOME="$DIR/home" PINNED_PASSWORD="$PASSWORD" "$SERVER" login "http://127.0.0.1:$NODE_PORT" "$EMAIL" \
  --name latency-node --config-dir "$DIR/node" >"$DIR/login.log" 2>&1
read -r TOKEN MACHINE < <(node -e 'const a=require(process.argv[1]); process.stdout.write(a.user_token+" "+a.machine_id+"\n")' "$DIR/node/account.json")

env HOME="$DIR/home" SHELL="$DIR/bin/shell" PATH="$LATENCY_PATH" BENCH_STATE="$DIR/state" \
  "$SERVER" node --config-dir "$DIR/node" >"$DIR/node.log" 2>&1 &
echo $! >"$DIR/node.pid"

set -- node "$REPO/scripts/latency-bench/bench.js" \
  --repo "$REPO" \
  --hub-ws "ws://127.0.0.1:$CLIENT_PORT/ws/client" \
  --token "$TOKEN" \
  --machine "$MACHINE" \
  --state "$DIR/state" \
  --work "$DIR/home/work" \
  --rtt-ms "$RTT_MS" \
  --iterations "$ITERATIONS" \
  --warmup "$WARMUP" \
  --client-stats "$DIR/client-proxy.stats" \
  --node-stats "$DIR/node-proxy.stats" \
  --out "$DIR/result.json"
if [ -n "$HEAVY" ]; then set -- "$@" --heavy; fi
"$@"
echo "result $DIR/result.json"
