#!/bin/bash
set -euo pipefail
set +m

REPO=$(cd "$(dirname "$0")/../.." && pwd)
DIR=${RECONNECT_DIR:-${TMPDIR:-/tmp}/pinned-reconnect-proof}
PG_PORT=${RECONNECT_PG_PORT:-55125}
HUB_PORT=${RECONNECT_HUB_PORT:-8795}
CLIENT_PORT=${RECONNECT_CLIENT_PORT:-8796}
NODE_PORT=${RECONNECT_NODE_PORT:-8797}
RTT_MS=${RTT_MS:-150}
EMAIL=reconnect@example.test
PASSWORD="reconnect password 1"
SERVER=${CARGO_TARGET_DIR:-$REPO/target}/release/pinned-server

case "$DIR" in
  *pinned-reconnect-proof*) ;;
  *) echo "refusing to use scratch dir $DIR" >&2; exit 1 ;;
esac

cleanup() {
  local status=$?
  set +e
  if [ -n "${CLIENT_PID:-}" ]; then kill "$CLIENT_PID" 2>/dev/null; wait "$CLIENT_PID" 2>/dev/null; fi
  if [ -f "$DIR/node.pid" ]; then kill "$(cat "$DIR/node.pid")" 2>/dev/null; wait "$(cat "$DIR/node.pid")" 2>/dev/null; fi
  if [ -f "$DIR/client-proxy.pid" ]; then kill "$(cat "$DIR/client-proxy.pid")" 2>/dev/null; fi
  if [ -f "$DIR/node-proxy.pid" ]; then kill "$(cat "$DIR/node-proxy.pid")" 2>/dev/null; fi
  if [ -f "$DIR/hub.pid" ]; then kill "$(cat "$DIR/hub.pid")" 2>/dev/null; fi
  if [ -d "$DIR/pg" ]; then pg_ctl -D "$DIR/pg" -m fast stop >/dev/null 2>&1; fi
  set -e
  return "$status"
}
trap cleanup EXIT

if [ -d "$DIR/pg" ]; then pg_ctl -D "$DIR/pg" -m fast stop >/dev/null 2>&1 || true; fi
rm -rf "$DIR"
mkdir -p "$DIR/bin" "$DIR/home/work" "$DIR/downloads" "$DIR/state" "$DIR/node"

cp "$REPO/scripts/latency-bench/fake-claude" "$DIR/bin/claude"
chmod +x "$DIR/bin/claude"
LATENCY_PATH="$DIR/bin:$(dirname "$(command -v node)"):/usr/bin:/bin"
printf '#!/bin/sh\nprintf "\\n__PINNED_PATH__%%s__PINNED_PATH__\\n" "%s"\n' "$LATENCY_PATH" >"$DIR/bin/shell"
chmod +x "$DIR/bin/shell"

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
disown
for _ in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:$HUB_PORT/" >/dev/null 2>&1 && break
  sleep 0.2
done
curl -fsS "http://127.0.0.1:$HUB_PORT/" >/dev/null

node "$REPO/scripts/latency-bench/proxy.js" --listen "127.0.0.1:$CLIENT_PORT" --upstream "127.0.0.1:$HUB_PORT" \
  --rtt-ms "$RTT_MS" --stats "$DIR/client-proxy.stats" >"$DIR/client-proxy.log" 2>&1 &
echo $! >"$DIR/client-proxy.pid"
disown
node "$REPO/scripts/latency-bench/proxy.js" --listen "127.0.0.1:$NODE_PORT" --upstream "127.0.0.1:$HUB_PORT" \
  --rtt-ms "$RTT_MS" --stats "$DIR/node-proxy.stats" >"$DIR/node-proxy.log" 2>&1 &
echo $! >"$DIR/node-proxy.pid"
disown
for port in "$CLIENT_PORT" "$NODE_PORT"; do
  ready=0
  for _ in $(seq 1 50); do
    if (echo >/dev/tcp/127.0.0.1/"$port") >/dev/null 2>&1; then ready=1; break; fi
    sleep 0.05
  done
  if [ "$ready" -ne 1 ]; then
    echo "port $port did not open" >&2
    cat "$DIR/client-proxy.log" "$DIR/node-proxy.log" >&2
    exit 1
  fi
done

curl -fsS "http://127.0.0.1:$HUB_PORT/api/signup" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" >/dev/null
env HOME="$DIR/home" PINNED_PASSWORD="$PASSWORD" "$SERVER" login "http://127.0.0.1:$NODE_PORT" "$EMAIL" \
  --name reconnect-node --config-dir "$DIR/node" >"$DIR/login.log" 2>&1
read -r TOKEN MACHINE < <(node -e 'const a=require(process.argv[1]); process.stdout.write(a.user_token+" "+a.machine_id+"\n")' "$DIR/node/account.json")

start_node() {
  env HOME="$DIR/home" SHELL="$DIR/bin/shell" PATH="$LATENCY_PATH" BENCH_STATE="$DIR/state" \
    "$SERVER" node --config-dir "$DIR/node" >"$DIR/node.log" 2>&1 &
  echo $! >"$DIR/node.pid"
  disown
}
stop_node() {
  local pid
  pid=$(cat "$DIR/node.pid")
  pkill -P "$pid" 2>/dev/null || true
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}
start_node

node "$REPO/scripts/latency-bench/reconnect-proof.js" \
  --repo "$REPO" \
  --hub-ws "ws://127.0.0.1:$CLIENT_PORT/ws/client" \
  --token "$TOKEN" \
  --machine "$MACHINE" \
  --state "$DIR/state" \
  --work "$DIR/home/work" \
  --phase "$DIR/phase" \
  --out "$DIR/result.json" \
  --rtt-ms "$RTT_MS" >"$DIR/client.out" 2>"$DIR/client.log" &
CLIENT_PID=$!
disown

wait_phase() {
  local want=$1
  local i
  for i in $(seq 1 400); do
    if [ "$(cat "$DIR/phase" 2>/dev/null || true)" = "$want" ]; then return 0; fi
    if ! kill -0 "$CLIENT_PID" 2>/dev/null; then
      echo "client exited waiting for $want" >&2
      cat "$DIR/client.log" >&2
      exit 1
    fi
    sleep 0.1
  done
  echo "timed out waiting for $want" >&2
  cat "$DIR/client.log" >&2
  exit 1
}

wait_phase cut
echo "client socket cut $(node -e 'process.stdout.write(String(Date.now()))') load $(sysctl -n vm.loadavg)"
kill -USR1 "$(cat "$DIR/client-proxy.pid")"
wait_phase restart
echo "node restart $(node -e 'process.stdout.write(String(Date.now()))') load $(sysctl -n vm.loadavg)"
stop_node
start_node
wait_phase done
echo "result $DIR/result.json"
cat "$DIR/client.out"
echo "client log:"
cat "$DIR/client.log"
echo "proxy log:"
cat "$DIR/client-proxy.log"
