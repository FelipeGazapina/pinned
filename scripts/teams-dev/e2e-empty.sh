#!/bin/sh
# From an empty directory to a running stack and back: up, status, the seeded users, a box opened and
# ended, Slack through the hub to the fake, reports through the hub to the fake GitHub, down, reset.
# One line per step, `ok <n> <what>` or `FAIL <n> <what>: <why>`; exits 1 on any FAIL.
#
#   TEAMS_DEV_BIN=target/debug/pinned-server [TEAMS_DEV_DIR=<fresh dir>] sh scripts/teams-dev/e2e-empty.sh
#
# Only the stack's own directory is used. Every process is stopped (by dev.sh down) before this ends.
set -u

# The stack dir is passed on, and then left out of this script's own environment, so that step 6 can
# tell a stack process (it carries TEAMS_DEV_DIR) from this script.
if [ -z "${E2E_CLEAN:-}" ]; then
  WORK=/home/boxuser/work
  [ -d "$WORK" ] || WORK=${TMPDIR:-/tmp}
  exec env -u TEAMS_DEV_DIR E2E_CLEAN=1 E2E_DIR="${TEAMS_DEV_DIR:-$(mktemp -d "$WORK/e2e-empty.XXXXXX")}" sh "$0" "$@"
fi

HERE=$(cd "$(dirname "$0")" && pwd)
: "${TEAMS_DEV_BIN:?set TEAMS_DEV_BIN to a built pinned-server}"
export TEAMS_DEV_BIN
DIR=$(realpath -m "$E2E_DIR")
HUB_PORT=${TEAMS_DEV_HUB_PORT:-8811} BOX_PORT=${TEAMS_DEV_BOX_PORT:-8812} SLACK_PORT=${TEAMS_DEV_SLACK_PORT:-8813} PG_PORT=${TEAMS_DEV_PG_PORT:-5611} GITHUB_PORT=${TEAMS_DEV_GITHUB_PORT:-8814}
for d in /usr/lib/postgresql/*/bin; do [ -x "$d/psql" ] && PATH=$d:$PATH; done
failed=0
# Subshells: an assignment before a function call can outlive the call, and would put the dir in this script's own environment.
dev() ( TEAMS_DEV_DIR=$DIR; export TEAMS_DEV_DIR; exec sh "$HERE/dev.sh" "$@" )
smoke() ( TEAMS_DEV_DIR=$DIR TEAMS_DEV_HUB="http://127.0.0.1:$HUB_PORT"; export TEAMS_DEV_DIR TEAMS_DEV_HUB; exec perl -e 'alarm shift; exec @ARGV' "$@" )
steps() ( TEAMS_DEV_DIR=$DIR TEAMS_DEV_HUB="http://127.0.0.1:$HUB_PORT" TEAMS_DEV_SLACK="http://127.0.0.1:$SLACK_PORT"; export TEAMS_DEV_DIR TEAMS_DEV_HUB TEAMS_DEV_SLACK; exec node "$HERE/e2e-steps.mjs" "$@" )
ok() { echo "ok $1 $2"; }
bad() { echo "FAIL $1 $2: $3"; failed=1; }

# started <pid>: when the process started, in clock ticks since boot.
started() { sed 's/^.*) //' "/proc/$1/stat" 2>/dev/null | cut -d' ' -f20; }
T0=$(started $$)

# stack_pids: processes the stack started, i.e. that carry its dir in their environment and began
# after this script did (a caller that exported TEAMS_DEV_DIR gave it to this script's neighbours too).
stack_pids() {
  for env in /proc/[0-9]*/environ; do
    [ -r "$env" ] || continue
    p=${env#/proc/}; p=${p%%/*}
    when=$(started "$p")
    [ -n "$when" ] && [ "$when" -gt $((T0 + 100)) ] || continue
    tr '\0' '\n' <"$env" 2>/dev/null | grep -qx "TEAMS_DEV_DIR=$DIR" && { p=${env#/proc/}; p=${p%%/*}; echo "$p $(tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null | cut -c1-80)"; }
  done
}
ports_free() {
  for port in $HUB_PORT $BOX_PORT $SLACK_PORT $PG_PORT $GITHUB_PORT; do
    ss -ltn 2>/dev/null | awk '{print $4}' | grep -q ":$port\$" && return 1
  done
  return 0
}
trap 'dev down >/dev/null 2>&1' EXIT

# 1
if out=$(dev up 2>&1); then
  if grep -q "DEV HUB: not production (time scale 60)" "$DIR/hub.log"; then ok 1 "dev.sh up, hub.log says DEV HUB: not production (time scale 60)"; else bad 1 "dev.sh up" "hub.log has no DEV HUB line"; fi
else bad 1 "dev.sh up" "$(echo "$out" | tail -3)"; fi

# 2
if out=$(dev status 2>&1) && [ "$(echo "$out" | grep -c '^up ')" -eq 5 ]; then ok 2 "status: hub, fake box, fake slack, fake github and postgres are up"; else bad 2 "status" "$out"; fi

# 3
if out=$(steps signin 2>&1); then ok 3 "the three seeded users sign in: $(echo "$out" | sed 's/^signed in //' | tr '\n' ' ')"; else bad 3 "sign in" "$out"; fi

# 4
# The passwords from before the reset, kept beside the stack dir (which `up` empties), mode 600.
OLDDIR=$(mktemp -d "$(dirname "$DIR")/e2e-old.XXXXXX")
OLD=$OLDDIR/accounts.json
cp "$DIR/accounts.json" "$OLD" && chmod 600 "$OLD"
if out=$(smoke 150 node "$HERE/smoke.mjs" open 2>&1) && echo "$out" | tail -1 | grep -q 'online=true'; then
  boxes=$(ls "$DIR/boxes" 2>/dev/null | wc -l)
  if [ "$boxes" -ge 1 ]; then
    if smoke 60 node "$HERE/smoke.mjs" disconnect >/dev/null 2>&1; then
      i=0
      while [ "$(ls "$DIR/boxes" 2>/dev/null | wc -l)" -gt 0 ] && [ $i -lt 100 ]; do i=$((i + 1)); sleep 0.3; done
      if [ "$(ls "$DIR/boxes" 2>/dev/null | wc -l)" -eq 0 ]; then ok 4 "a box came online ($(echo "$out" | tail -1 | cut -d' ' -f1)) and its directory is gone after disconnect"; else bad 4 "disconnect" "the box directory is still there"; fi
    else bad 4 "disconnect" "smoke.mjs disconnect failed"; fi
  else bad 4 "open" "online, but no box directory under $DIR/boxes"; fi
else bad 4 "open a box" "$(echo "$out" | tail -2)"; fi

# 5
if out=$(steps slack 2>&1); then ok 5 "Slack through the hub reaches the fake ($(echo "$out" | head -1 | sed 's/^the fake Slack saw //')); a non-dev token is refused"; else bad 5 "Slack through the hub" "$out"; fi

# 6
if grep -q "reporting is off\|PINNED_REPORTS_[A-Z_]* is ignored" "$DIR/hub.log"; then bad 6 "reports" "hub.log says reporting is off or an override is ignored"
elif out=$(steps reports 2>&1); then ok 6 "$out"; else bad 6 "reports through the hub" "$out"; fi

# 7
dev down >/dev/null 2>&1
left=$(stack_pids | tr '\n' ';')
if [ -z "$left" ] && ports_free; then ok 7 "down: no process carries the stack dir and ports $HUB_PORT-$SLACK_PORT, $GITHUB_PORT and $PG_PORT are free"; else bad 7 "down" "processes: ${left:-none}; ports in use: $(ss -ltn | awk '{print $4}' | grep -E ":($HUB_PORT|$BOX_PORT|$SLACK_PORT|$PG_PORT|$GITHUB_PORT)\$" | tr '\n' ' ')"; fi

# 8
if out=$(dev reset 2>&1); then
  rows=$(psql -h 127.0.0.1 -p "$PG_PORT" -U pinned -d pinned -Atc 'select count(*) from accounts' 2>&1)
  held=$(psql -h 127.0.0.1 -p "$PG_PORT" -U pinned -d pinned -Atc 'select (select count(*) from boxes) + (select count(*) from machines)' 2>&1)
  if [ "$rows" = 3 ] && [ "$held" = 0 ] && ! ls "$DIR/boxes"/* >/dev/null 2>&1 && out=$(steps empty "$OLD" 2>&1); then ok 8 "reset: only the three seeded accounts, no box, no Slack"; else bad 8 "reset" "accounts=$rows boxes+machines=$held $out"; fi
else bad 8 "reset" "$(echo "$out" | tail -3)"; fi

dev down >/dev/null 2>&1
rm -rf "$OLDDIR"
[ $failed -eq 0 ]
