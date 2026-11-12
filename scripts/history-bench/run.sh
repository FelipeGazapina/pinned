#!/bin/sh
# Starts the web sweep's stack, runs bench.js against it, and stops the stack, all in the foreground.
#   scripts/history-bench/run.sh [turns=2000] [runs=3]
set -eu
REPO=$(cd "$(dirname "$0")/../.." && pwd)
export SWEEP_DIR=${SWEEP_DIR:-${TMPDIR:-/tmp}/pinned-history-bench}
trap '"$REPO/scripts/web-sweep/stack.sh" stop' EXIT
"$REPO/scripts/web-sweep/stack.sh" start
node "$REPO/scripts/history-bench/bench.js" "$@"
