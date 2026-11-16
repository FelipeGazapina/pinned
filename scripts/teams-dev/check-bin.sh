#!/bin/sh
# check-bin.sh <pinned-server>: exits 0 when the binary is a debug build, 2 (with a plain message) for
# anything else. The dev stack points its Box calls at a fake only in a build it can vouch for: a release
# build could send them to the real Upstash.
#
# The binary is asked by starting `hub --dev` with an empty environment (no box or Slack address, no
# key), a database that is not there and port 0. The dev check (crates/server/src/dev.rs `check`) refuses
# that start before it reads the database or binds anything: a release build says it is a release build,
# a debug one says UPSTASH_BOX_BASE_URL is not set. Any other answer (a binary that does not know --dev,
# or one that went on to run) is refused too, so this fails closed.
set -u

bin=${1:-}
if [ -z "$bin" ] || [ ! -f "$bin" ] || [ ! -x "$bin" ]; then
  echo "no pinned-server to check: build one (cargo build -p pinned-server) or set TEAMS_DEV_BIN" >&2
  exit 2
fi
out=$(env -i PATH="$PATH" perl -e 'alarm 10; exec @ARGV' "$bin" hub --dev --db postgres://probe@127.0.0.1:1/probe --listen 127.0.0.1:0 --ui /nonexistent --downloads /nonexistent 2>&1 </dev/null)
code=$?
case $out in
  *"release build"*)
    echo "the pinned-server is not a debug build, so the dev stack will not start it: a release build could reach the real Upstash box. Build it with cargo build -p pinned-server, without --release." >&2
    exit 2 ;;
  *"UPSTASH_BOX_BASE_URL is not set"*)
    if [ "$code" -eq 2 ]; then echo "the pinned-server is a debug build"; exit 0; fi ;;
esac
echo "the pinned-server does not answer the dev check as a debug build of this checkout does (exit $code): rebuild it with cargo build -p pinned-server" >&2
exit 2
