#!/bin/sh
# BOX ONLY. Do not run this on a real computer (a laptop, a Mac mini, a server with a real pinned).
# The fake box runner checks only the command line it is given: a path the command builds while it
# runs (a variable, a script file, an interpreter) slips past its "stay inside the stack dir" check.
# The real isolation is the Upstash box the stack runs in, not this guard.
#
# One command for a local pinned development stack with empty data, three invented accounts and a
# fake Upstash Box runner, for exercising Teams flows end to end. See scripts/teams-dev/README.md.
#
#   scripts/teams-dev/dev.sh up|down|status|reset|urls|seed
#
# TEAMS_DEV_DIR (default: a fresh mktemp dir, recorded in .dev/teams-dev.dir) holds every file the
# stack writes; every child runs with HOME, TMPDIR and CLAUDE_CONFIG_DIR inside it and with the
# developer's tokens unset. TEAMS_DEV_BIN=<pinned-server> skips the cargo build; the binary must be a
# debug build (check-bin.sh asks it, and exits 2 for anything else). Ports:
# TEAMS_DEV_HUB_PORT 8811, TEAMS_DEV_BOX_PORT 8812, TEAMS_DEV_SLACK_PORT 8813, TEAMS_DEV_PG_PORT 5611,
# TEAMS_DEV_GITHUB_PORT 8814.
# The hub always starts with --dev and PINNED_DEV_TIME_SCALE=${TEAMS_DEV_TIME_SCALE:-60}, and files
# reports only on the fake GitHub (`pinned-server fake-github`), with a fake- token only it accepts.
# The stack refuses (exit 2) a dir that is, or is inside, a real pinned home: ~/.pinned,
# ~/.config/pinned, the desktop app's data folder, /workspace/home, or the home folder or / itself.
set -eu

# The developer's real home, taken before scrub_env points HOME at the stack.
REAL_HOME=${HOME:-}

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
GITHUB_PORT=${TEAMS_DEV_GITHUB_PORT:-8814}
GITHUB=http://127.0.0.1:$GITHUB_PORT
HUB=http://127.0.0.1:$HUB_PORT
BOX=http://127.0.0.1:$BOX_PORT
RECORD=$REPO/.dev/teams-dev.dir

# real_path <path>: the absolute path with symlinks followed, for a path that need not exist yet
# (its nearest existing parent is resolved). Creates nothing.
real_path() {
  case $1 in /*) set -- "$1" ;; *) set -- "$PWD/$1" ;; esac
  if realpath -m / >/dev/null 2>&1; then realpath -m -- "$1"; return; fi
  p=$1 rest=
  while [ ! -d "$p" ]; do
    case $(basename "$p") in .|..) echo "refusing a dir path with a . or .. in a part that does not exist yet" >&2; exit 2 ;; esac
    rest=/$(basename "$p")$rest
    p=$(dirname "$p")
  done
  echo "$(cd "$p" && pwd -P)$rest"
}

# refuse_real_home <path>: exits 2, saying only which kind of folder, when the path is or is inside
# a folder pinned keeps real data in. Called before anything is created.
refuse_real_home() {
  target=$(real_path "$1")
  for home in "$REAL_HOME" "$(getent passwd "$(id -u)" 2>/dev/null | cut -d: -f6)"; do
    [ -n "$home" ] || continue
    home=$(real_path "$home")
    for kind in ".pinned:a pinned data folder (~/.pinned)" ".config/pinned:a pinned config folder (~/.config/pinned)" "Library/Application Support/dev.personal-harness.pinned:the pinned desktop app's data folder"; do
      folder=$(real_path "$home/${kind%%:*}")
      case $target in "$folder"|"$folder"/*) echo "refusing to run the dev stack in ${kind#*:}" >&2; exit 2 ;; esac
    done
    [ "$target" = "$home" ] && { echo "refusing to run the dev stack in your home folder itself" >&2; exit 2; }
  done
  case $target in
    /workspace/home|/workspace/home/*) echo "refusing to run the dev stack in a box's persisted home folder" >&2; exit 2 ;;
    /) echo "refusing to run the dev stack in the filesystem root" >&2; exit 2 ;;
  esac
}

# The dir of this stack: $TEAMS_DEV_DIR, else the one `up` recorded, else (for `up`) a new one.
# The dir is checked by refuse_real_home before it is created or touched.
resolve_dir() {
  if [ -n "${TEAMS_DEV_DIR:-}" ]; then DIR=$TEAMS_DEV_DIR
  elif [ -f "$RECORD" ]; then DIR=$(cat "$RECORD")
  elif [ "$1" = up ]; then
    refuse_real_home "${TMPDIR:-/tmp}"
    DIR=$(mktemp -d "${TMPDIR:-/tmp}/teams-dev.XXXXXX")
  else echo "no dev stack recorded in $RECORD: run $0 up" >&2; exit 1
  fi
  [ -n "$DIR" ] || { echo "the stack dir is empty" >&2; exit 2; }
  refuse_real_home "$DIR"
  DIR=$(real_path "$DIR")
  case $1 in up|reset|seed) mkdir -p "$DIR" ;; esac
}

# alive <pid-file>: the pid file's process runs and was started by this stack: it carries the dir in
# its environment (the fake box gives its boxes' commands TEAMS_DEV_DIR too), so a reused pid, or
# one that names some other process, is never signalled.
alive() {
  [ -f "$1" ] || return 1
  pid=$(cat "$1")
  case $pid in ''|*[!0-9]*) return 1 ;; esac
  [ -d "/proc/$pid" ] || return 1
  tr '\0' '\n' 2>/dev/null <"/proc/$pid/environ" | grep -qx "TEAMS_DEV_DIR=$DIR"
}

# Own pid files only: the stack's (hub.pid, fake-box.pid, fake-slack.pid, fake-github.pid) and its box nodes' node.pid.
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
  unset PINNED_REPORTS_GITHUB_TOKEN PINNED_REPORTS_GITHUB_API PINNED_REPORTS_REPO PINNED_REPORTS_POLL_SECS
  mkdir -p "$DIR/home/.claude" "$DIR/tmp"
  export HOME="$DIR/home" TMPDIR="$DIR/tmp" CLAUDE_CONFIG_DIR="$DIR/home/.claude" TEAMS_DEV_DIR="$DIR" TEAMS_DEV_HUB="$HUB"
  # Boxes are the fake's, always: a key in the developer's shell never reaches a real Upstash.
  export UPSTASH_BOX_BASE_URL="$BOX" PINNED_SLACK_API_BASE="http://127.0.0.1:$SLACK_PORT/api/"
}

up() {
  if alive "$DIR/hub.pid"; then echo "the stack is already up (dir $DIR)"; urls; return 0; fi
  stop_all
  # Free ports are the caller's to pick: say which one is taken rather than start half a stack.
  for port in $HUB_PORT $BOX_PORT $SLACK_PORT $PG_PORT $GITHUB_PORT; do
    if ss -ltn 2>/dev/null | awk '{print $4}' | grep -q ":$port\$"; then echo "port $port is in use: set TEAMS_DEV_*_PORT" >&2; exit 1; fi
  done
  if [ -n "${TEAMS_DEV_BIN:-}" ]; then SERVER=$TEAMS_DEV_BIN
  else
    ${CARGO:-cargo} build -p pinned-server --manifest-path "$REPO/Cargo.toml" >&2
    SERVER=${CARGO_TARGET_DIR:-$REPO/target}/debug/pinned-server
  fi
  [ -x "$SERVER" ] || { echo "no pinned-server at $SERVER" >&2; exit 1; }
  # Everything below is a fresh stack: the data dir is emptied, so a new hub key, database and accounts.
  # Only a dir that is empty or was a stack's is emptied, never somebody's folder.
  if [ ! -e "$DIR/.teams-dev-stack" ] && [ -n "$(ls -A "$DIR")" ]; then
    echo "refusing to empty a folder that is not an empty dir or a dev stack's" >&2; exit 2
  fi
  # Only a debug build is started: it is asked, before anything is emptied, created or started.
  sh "$HERE/check-bin.sh" "$SERVER" || exit 2
  find "$DIR" -mindepth 1 -maxdepth 1 ! -name '*.log' -exec rm -rf {} + 2>/dev/null || true
  : >"$DIR/.teams-dev-stack"
  chmod 700 "$DIR"
  mkdir -p "$REPO/.dev" "$DIR/downloads"
  echo "$DIR" >"$RECORD"
  cp "$SERVER" "$DIR/downloads/pinned-server-aarch64-unknown-linux-musl"
  scrub_env

  pg_start "$DIR/pg" "$PG_PORT" pinned

  node "$HERE/fake-box" "$BOX_PORT" >"$DIR/fake-box.log" 2>&1 &
  echo $! >"$DIR/fake-box.pid"
  wait_for "curl -fs -H 'X-Box-Api-Key: box_dev-probe' $BOX/v2/box" "the fake box"

  node "$HERE/fake-slack" --port "$SLACK_PORT" --log "$DIR/fake-slack.log.jsonl" >"$DIR/fake-slack.log" 2>&1 &
  echo $! >"$DIR/fake-slack.pid"
  wait_for "curl -s -o /dev/null http://127.0.0.1:$SLACK_PORT/" "the fake Slack"

  "$SERVER" fake-github --listen "127.0.0.1:$GITHUB_PORT" >"$DIR/fake-github.log" 2>&1 &
  echo $! >"$DIR/fake-github.pid"
  wait_for "curl -s -o /dev/null $GITHUB/" "the fake GitHub"

  export PINNED_DEV_TIME_SCALE=${TEAMS_DEV_TIME_SCALE:-60}
  # Reports go to the fake GitHub only. The token is a placeholder the fake takes and GitHub never would;
  # the hub refuses, under --dev, any reports API that is not on this machine.
  export PINNED_REPORTS_GITHUB_API="$GITHUB" PINNED_REPORTS_GITHUB_TOKEN=fake-teams-dev
  "$SERVER" hub --db "postgres://pinned@127.0.0.1:$PG_PORT/pinned" --listen "127.0.0.1:$HUB_PORT" --ui "$REPO/ui" \
    --downloads "$DIR/downloads" --key-file "$DIR/hub.key" --public-url "$HUB" --signup open --dev >"$DIR/hub.log" 2>&1 &
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
  component "fake slack http://127.0.0.1:$SLACK_PORT" "$DIR/fake-slack.pid" "curl -s -o /dev/null http://127.0.0.1:$SLACK_PORT/"
  component "fake github $GITHUB" "$DIR/fake-github.pid" "curl -s -o /dev/null $GITHUB/"
  if pg_isready -q -h 127.0.0.1 -p "$PG_PORT"; then echo "up    postgres   127.0.0.1:$PG_PORT"; else echo "down  postgres   127.0.0.1:$PG_PORT"; down=1; fi
  echo "dir   $DIR"
  [ -z "$down" ]
}

urls() {
  echo "hub        $HUB   (sign in with a user below)"
  echo "fake box   $BOX   (keys: box_dev-<anything>)"
  echo "fake slack http://127.0.0.1:$SLACK_PORT   (a team or account connects Slack with xoxb-dev-<anything>)"
  echo "fake github $GITHUB   (the hub files reports here)"
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
