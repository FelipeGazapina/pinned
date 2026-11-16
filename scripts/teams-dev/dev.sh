#!/bin/sh
# One command for a local pinned development stack with empty data, three invented accounts and a
# fake Upstash Box runner, for exercising Teams flows end to end. See scripts/teams-dev/README.md.
#
#   scripts/teams-dev/dev.sh up|down|status|reset|urls|seed
#
# TEAMS_DEV_DIR (default: a fresh mktemp dir, recorded in .dev/teams-dev.dir) holds every file the
# stack writes; every child runs with HOME, TMPDIR and CLAUDE_CONFIG_DIR inside it and with the
# developer's tokens unset. TEAMS_DEV_BIN=<pinned-server> skips the cargo build. Ports:
# TEAMS_DEV_HUB_PORT 8811, TEAMS_DEV_BOX_PORT 8812, TEAMS_DEV_SLACK_PORT 8813, TEAMS_DEV_PG_PORT 5611.
# TEAMS_DEV_TIME_SCALE (60) is PINNED_DEV_TIME_SCALE when the hub has a --dev flag.
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
. "$REPO/scripts/web-sweep/lib.sh"
HERE=$REPO/scripts/teams-dev
# Debian and Homebrew keep initdb, pg_ctl and psql off the default PATH.
command -v pg_ctl >/dev/null 2>&1 || for d in /usr/lib/postgresql/*/bin /opt/homebrew/opt/postgresql*/bin /usr/local/opt/postgresql*/bin; do
  [ -x "$d/pg_ctl" ] && { PATH=$d:$PATH; break; }
done
HUB_PORT=${TEAMS_DEV_HUB_PORT:-8811}
BOX_PORT=${TEAMS_DEV_BOX_PORT:-8812}
SLACK_PORT=${TEAMS_DEV_SLACK_PORT:-8813}
PG_PORT=${TEAMS_DEV_PG_PORT:-5611}
HUB=http://127.0.0.1:$HUB_PORT
BOX=http://127.0.0.1:$BOX_PORT
RECORD=$REPO/.dev/teams-dev.dir

# The dir of this stack: $TEAMS_DEV_DIR, else the one `up` recorded, else (for `up`) a new one.
resolve_dir() {
  if [ -n "${TEAMS_DEV_DIR:-}" ]; then DIR=$TEAMS_DEV_DIR
  elif [ -f "$RECORD" ]; then DIR=$(cat "$RECORD")
  elif [ "$1" = up ]; then DIR=$(mktemp -d "${TMPDIR:-/tmp}/teams-dev.XXXXXX")
  else echo "no dev stack recorded in $RECORD: run $0 up" >&2; exit 1
  fi
  mkdir -p "$DIR"
  DIR=$(cd "$DIR" && pwd)
}

# alive <pid-file>: the pid file's process runs and was started by this stack (it carries the dir in
# its environment, or, for a box's node, runs pinned-server), so a reused pid is never signalled.
alive() {
  [ -f "$1" ] || return 1
  pid=$(cat "$1")
  [ -d "/proc/$pid" ] || return 1
  tr '\0' '\n' 2>/dev/null <"/proc/$pid/environ" | grep -qx "TEAMS_DEV_DIR=$DIR" && return 0
  case $1 in "$DIR"/boxes/*) grep -qs pinned-server "/proc/$pid/cmdline" ;; *) return 1 ;; esac
}

# Own pid files only: the stack's (hub.pid, fake-box.pid, fake-slack.pid) and its box nodes' node.pid.
pid_files() {
  ls "$DIR"/*.pid "$DIR"/boxes/*/home/.pinned/node.pid 2>/dev/null || true
}

stop_all() {
  files=$(pid_files)
  for f in $files; do alive "$f" && kill "$(cat "$f")" 2>/dev/null || true; done
  i=0
  while [ $i -lt 50 ]; do
    busy=
    for f in $files; do alive "$f" && busy=1; done
    [ -z "$busy" ] && break
    i=$((i + 1)); sleep 0.2
  done
  for f in $files; do alive "$f" && kill -9 "$(cat "$f")" 2>/dev/null || true; rm -f "$f"; done
  pg_stop "$DIR/pg"
  if pg_isready -q -h 127.0.0.1 -p "$PG_PORT" && [ -f "$DIR/pg/postmaster.pid" ]; then echo "postgres in $DIR/pg is still running: stop it with pg_ctl -D $DIR/pg -m fast stop" >&2; fi
}

# The environment of every child: its own HOME, TMPDIR and Claude config, and none of the developer's tokens.
scrub_env() {
  for name in $(env | cut -d= -f1 | grep '^ANTHROPIC_' || true); do unset "$name"; done
  unset CLAUDE_CODE_OAUTH_TOKEN GH_TOKEN GITHUB_TOKEN PINNED_OPENCODE_GO_KEY UPSTASH_BOX_API_KEY
  mkdir -p "$DIR/home/.claude" "$DIR/tmp"
  export HOME="$DIR/home" TMPDIR="$DIR/tmp" CLAUDE_CONFIG_DIR="$DIR/home/.claude" TEAMS_DEV_DIR="$DIR" TEAMS_DEV_HUB="$HUB"
  # Boxes are the fake's, always: a key in the developer's shell never reaches a real Upstash.
  export UPSTASH_BOX_BASE_URL="$BOX" PINNED_SLACK_API_BASE="http://127.0.0.1:$SLACK_PORT/api/"
}

up() {
  if alive "$DIR/hub.pid"; then echo "the stack is already up (dir $DIR)"; urls; return 0; fi
  stop_all
  # Free ports are the caller's to pick: say which one is taken rather than start half a stack.
  for port in $HUB_PORT $BOX_PORT $SLACK_PORT $PG_PORT; do
    if ss -ltn 2>/dev/null | awk '{print $4}' | grep -q ":$port\$"; then echo "port $port is in use: set TEAMS_DEV_*_PORT" >&2; exit 1; fi
  done
  if [ -n "${TEAMS_DEV_BIN:-}" ]; then SERVER=$TEAMS_DEV_BIN
  else
    ${CARGO:-cargo} build -p pinned-server --manifest-path "$REPO/Cargo.toml" >&2
    SERVER=${CARGO_TARGET_DIR:-$REPO/target}/debug/pinned-server
  fi
  [ -x "$SERVER" ] || { echo "no pinned-server at $SERVER" >&2; exit 1; }
  # Everything below is a fresh stack: the data dir is emptied, so a new hub key, database and accounts.
  find "$DIR" -mindepth 1 -maxdepth 1 ! -name '*.log' -exec rm -rf {} + 2>/dev/null || true
  chmod 700 "$DIR"
  mkdir -p "$REPO/.dev" "$DIR/downloads"
  echo "$DIR" >"$RECORD"
  cp "$SERVER" "$DIR/downloads/pinned-server-aarch64-unknown-linux-musl"
  scrub_env

  pg_start "$DIR/pg" "$PG_PORT" pinned

  node "$HERE/fake-box" "$BOX_PORT" >"$DIR/fake-box.log" 2>&1 &
  echo $! >"$DIR/fake-box.pid"
  wait_for "curl -fs -H 'X-Box-Api-Key: box_dev-probe' $BOX/v2/box" "the fake box"

  if [ -f "$HERE/fake-slack" ]; then
    node "$HERE/fake-slack" "$SLACK_PORT" >"$DIR/fake-slack.log" 2>&1 &
    echo $! >"$DIR/fake-slack.pid"
    wait_for "curl -s -o /dev/null http://127.0.0.1:$SLACK_PORT/" "the fake Slack"
  fi

  dev=
  if "$SERVER" hub --help 2>&1 | grep -q -- '--dev'; then
    dev=--dev
    export PINNED_DEV_TIME_SCALE=${TEAMS_DEV_TIME_SCALE:-60}
  fi
  # shellcheck disable=SC2086
  "$SERVER" hub --db "postgres://pinned@127.0.0.1:$PG_PORT/pinned" --listen "127.0.0.1:$HUB_PORT" --ui "$REPO/ui" \
    --downloads "$DIR/downloads" --key-file "$DIR/hub.key" --public-url "$HUB" --signup open $dev >"$DIR/hub.log" 2>&1 &
  echo $! >"$DIR/hub.pid"
  wait_for "curl -fs $HUB/" "the hub"

  node "$HERE/seed.mjs"
  echo
  urls
}

component() {
  # component <name> <pid-file> <health command>
  if alive "$2" && eval "$3" >/dev/null 2>&1; then echo "up    $1"; else echo "down  $1"; down=1; fi
}

status() {
  down=
  component "hub        $HUB" "$DIR/hub.pid" "curl -fs $HUB/"
  component "fake box   $BOX" "$DIR/fake-box.pid" "curl -fs -H 'X-Box-Api-Key: box_dev-probe' $BOX/v2/box"
  if [ -f "$HERE/fake-slack" ]; then component "fake slack http://127.0.0.1:$SLACK_PORT" "$DIR/fake-slack.pid" "curl -s -o /dev/null http://127.0.0.1:$SLACK_PORT/"; fi
  if pg_isready -q -h 127.0.0.1 -p "$PG_PORT"; then echo "up    postgres   127.0.0.1:$PG_PORT"; else echo "down  postgres   127.0.0.1:$PG_PORT"; down=1; fi
  echo "dir   $DIR"
  [ -z "$down" ]
}

urls() {
  echo "hub        $HUB   (sign in with a user below)"
  echo "fake box   $BOX   (keys: box_dev-<anything>)"
  [ -f "$HERE/fake-slack" ] && echo "fake slack http://127.0.0.1:$SLACK_PORT"
  echo "users      (passwords: $DIR/accounts.json, mode 600)"
  if [ -f "$DIR/accounts.json" ]; then node -e 'for (const e of Object.keys(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")))) console.log("  " + e)' "$DIR/accounts.json"; fi
}

cmd=${1:-}
case $cmd in
  up|down|status|reset|urls|seed) ;;
  *) echo "usage: $0 up|down|status|reset|urls|seed" >&2; exit 2 ;;
esac
resolve_dir "$cmd"
case $cmd in
  up) up ;;
  down) stop_all; echo "stack down (dir $DIR kept: $0 reset starts over)" ;;
  status) status ;;
  urls) urls ;;
  seed) scrub_env; node "$HERE/seed.mjs" ;;
  reset) stop_all; up ;;
esac
