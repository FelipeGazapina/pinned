#!/bin/sh
# Starts a throwaway stack for the web sweep: Postgres, a hub serving this checkout's ui/, an
# account, and a paired node whose `claude` is scripts/web-sweep/fake-claude and `gh` fake-gh.
#
#   scripts/web-sweep/stack.sh start   # prints the hub URL and the account
#   scripts/web-sweep/stack.sh stop
#
# SWEEP_DIR (default $TMPDIR/pinned-web-sweep) holds every file; SWEEP_PG_PORT (5497) and
# SWEEP_HUB_PORT (8797) pick the ports; CARGO runs cargo (default `cargo`). Nothing outside
# SWEEP_DIR is written: the node runs with HOME set inside it.
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
DIR=${SWEEP_DIR:-${TMPDIR:-/tmp}/pinned-web-sweep}
PG_PORT=${SWEEP_PG_PORT:-5497}
HUB_PORT=${SWEEP_HUB_PORT:-8797}
HUB=http://127.0.0.1:$HUB_PORT
EMAIL=sweep@example.test
PASSWORD="sweep password 1"
SERVER=${CARGO_TARGET_DIR:-$REPO/target}/debug/pinned-server

stop() {
  for name in node hub; do
    [ -f "$DIR/$name.pid" ] && kill "$(cat "$DIR/$name.pid")" 2>/dev/null || true
    rm -f "$DIR/$name.pid"
  done
  [ -d "$DIR/pg" ] && pg_ctl -D "$DIR/pg" -m fast stop >/dev/null 2>&1 || true
}

wait_for() {
  i=0
  until eval "$1" >/dev/null 2>&1; do
    i=$((i + 1))
    [ $i -gt 100 ] && { echo "timed out waiting for $2" >&2; exit 1; }
    sleep 0.2
  done
}

start() {
  stop
  rm -rf "$DIR"
  mkdir -p "$DIR/bin" "$DIR/home/work" "$DIR/downloads"
  ${CARGO:-cargo} build -p pinned-server --manifest-path "$REPO/Cargo.toml" >&2

  export LC_ALL=C
  initdb -U pinned --auth=trust -D "$DIR/pg" >/dev/null
  pg_ctl -D "$DIR/pg" -l "$DIR/pg.log" -o "-p $PG_PORT -c listen_addresses=127.0.0.1 -c unix_socket_directories=" start >/dev/null
  wait_for "psql -h 127.0.0.1 -p $PG_PORT -U pinned -d postgres -c 'select 1'" postgres
  psql -h 127.0.0.1 -p "$PG_PORT" -U pinned -d postgres -qc "create database pinned"

  "$SERVER" hub --db "postgres://pinned@127.0.0.1:$PG_PORT/pinned" --listen "127.0.0.1:$HUB_PORT" --ui "$REPO/ui" \
    --downloads "$DIR/downloads" --key-file "$DIR/hub.key" --public-url "$HUB" --signup open >"$DIR/hub.log" 2>&1 &
  echo $! >"$DIR/hub.pid"
  wait_for "curl -fs $HUB/" hub

  curl -fs "$HUB/api/signup" -H 'content-type: application/json' -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" >/dev/null

  # The node asks the login shell for PATH; this shell answers with the sweep's, fake claude first.
  cp "$REPO/scripts/web-sweep/fake-claude" "$DIR/bin/claude"
  cp "$REPO/scripts/web-sweep/fake-gh" "$DIR/bin/gh"
  cp "$REPO/scripts/web-sweep/fake-npx" "$DIR/bin/npx"
  cp "$REPO/scripts/web-sweep/fake-adb" "$DIR/bin/adb"
  cp "$REPO/scripts/web-sweep/fake-adb" "$DIR/bin/emulator"
  printf '#!/bin/sh\nshift\nexec /bin/sh -c "$1"\n' >"$DIR/bin/shell"
  # Opening an editor logs what it would open, and a Zed.app gives the node an editor to list.
  printf '#!/bin/sh\necho "$@" >>"$HOME/open.log"\n' >"$DIR/bin/open"
  mkdir -p "$DIR/home/Applications/Zed.app"
  chmod +x "$DIR/bin/claude" "$DIR/bin/gh" "$DIR/bin/npx" "$DIR/bin/adb" "$DIR/bin/emulator" "$DIR/bin/shell" "$DIR/bin/open"
  for p in alpha beta gamma; do
    git -C "$DIR/home/work" init -q "$p"
    git -C "$DIR/home/work/$p" remote add origin "https://github.com/example/$p.git"
  done
  touch "$DIR/home/work/gamma/.untrusted"
  env HOME="$DIR/home" PINNED_PASSWORD="$PASSWORD" "$SERVER" login "$HUB" "$EMAIL" --name sweep-node --config-dir "$DIR/node" >/dev/null
  env HOME="$DIR/home" SHELL="$DIR/bin/shell" PATH="$DIR/bin:$PATH" PINNED_PR_WATCH_SECS=2 "$SERVER" node --config-dir "$DIR/node" >"$DIR/node.log" 2>&1 &
  echo $! >"$DIR/node.pid"

  echo "hub $HUB"
  echo "account $EMAIL / $PASSWORD"
  echo "dir $DIR"
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  *) echo "usage: $0 start|stop" >&2; exit 2 ;;
esac
