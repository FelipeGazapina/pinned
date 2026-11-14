#!/usr/bin/env bash
# Runs one local hub binary under hub_load and stops it, all in the foreground.
#
#   hub_load.sh <pinned-server binary> <port> <postgres db name> <out dir> [hub_load flags...]
#
# Needs a local Postgres on 127.0.0.1:5499 (user hub, trust auth) and the hub_load example built
# (cargo build --release -p pinned-server --example hub_load). Set WRAP to run the hub under a
# profiler, e.g. WRAP="heaptrack -o <file>". Everything runs with its own HOME, fake accounts only.
set -euo pipefail
bin=$1 port=$2 db=$3 out=$4
shift 4
here=$(cd "$(dirname "$0")/../../.." && pwd)
load=$here/target/release/examples/hub_load
mkdir -p "$out"
home=$(mktemp -d /tmp/h.XXXX)
mkdir -p "$home/ui" "$home/downloads"
/usr/lib/postgresql/15/bin/createdb -h 127.0.0.1 -p 5499 -U hub "$db" 2>/dev/null || true
# setsid: the hub leads its own process group, so the stop below takes every process it started.
HOME=$home setsid ${WRAP:-} "$bin" hub --db "postgres://hub@127.0.0.1:5499/$db" --listen "127.0.0.1:$port" \
  --ui "$home/ui" --downloads "$home/downloads" --key-file "$home/hub.key" --signup open --trusted-proxy off \
  >"$out/hub.log" 2>&1 &
leader=$!
stop() {
  kill -s TERM -- "-$leader" 2>/dev/null || true
  for _ in $(seq $((${STOP_WAIT:-10} * 5))); do kill -0 "$leader" 2>/dev/null || break; sleep 0.2; done
  kill -s KILL -- "-$leader" 2>/dev/null || true
  wait "$leader" 2>/dev/null || true
}
trap stop EXIT
for _ in $(seq 100); do
  (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null && break
  sleep 0.2
done
# The hub's own pid: the leader itself, or the binary the wrapper started.
pid=$(pgrep -g "$leader" -f "$(basename "$bin") hub" | tail -1)
echo "hub pid $pid (group $leader)" | tee "$out/pid"
"$load" --hub "127.0.0.1:$port" --pid "$pid" "$@" | tee "$out/load.log"
grep census "$out/hub.log" | tail -5 || true
