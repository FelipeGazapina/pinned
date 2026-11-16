#!/bin/sh
# check-bin.sh must accept a pinned-server that is a debug build and refuse every other answer, with a
# plain message, and the binary must never see a key for a real box. The binaries here are stubs.
#
#   sh scripts/teams-dev/check-bin.test.sh
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
WORK=${GUARD_TEST_DIR:-/home/boxuser/work}
[ -d "$WORK" ] || WORK=${TMPDIR:-/tmp}
T=$(mktemp -d "$WORK/check-bin.XXXXXX") || exit 1
trap 'rm -rf "$T"' EXIT
failed=0
fail() { echo "FAIL $1" >&2; failed=1; }

# stub <name> <exit code> <message>: a binary that prints the message on stderr and exits.
stub() {
  printf '#!/bin/sh\necho "$UPSTASH_BOX_BASE_URL:$PINNED_SLACK_API_BASE" >"%s/seen-env"\nenv >"%s/seen-all"\necho "args: $*" >"%s/seen-args"\necho "%s" >&2\nexit %s\n' "$T" "$T" "$T" "$3" "$2" >"$T/$1"
  chmod +x "$T/$1"
}
# run <what> <bin> <want exit> <want message part>
run() {
  out=$(UPSTASH_BOX_BASE_URL=https://real-box.example.test PINNED_SLACK_API_BASE=https://slack.example.test/api UPSTASH_BOX_API_KEY=box_real-s3cret sh "$HERE/check-bin.sh" "$2" 2>&1); code=$?
  [ "$code" -eq "$3" ] || fail "$1: exit $code, wanted $3: $out"
  case $out in *"$4"*) ;; *) fail "$1: the message lacks \"$4\": $out" ;; esac
  case $out in *s3cret*|*real-box*) fail "$1: the message leaks an environment value: $out" ;; esac
  [ $failed -eq 0 ] && echo "ok $1: $out"
}

stub debug 2 "pinned-server: UPSTASH_BOX_BASE_URL is not set"
stub release 2 "pinned-server: this is a release build: --dev needs a debug build (cargo build -p pinned-server, without --release)"
stub old-binary 1 "pinned-server: unknown option --dev"
stub runs-on 0 "hub started"
stub other-refusal 2 "pinned-server: --db host is not loopback"

run "a debug build is accepted" "$T/debug" 0 "debug build"
run "a release build is refused" "$T/release" 2 "not a debug build"
run "a binary that does not know --dev is refused" "$T/old-binary" 2 "does not answer"
run "a binary that starts a hub is refused" "$T/runs-on" 2 "does not answer"
run "an answer that is not the debug one is refused" "$T/other-refusal" 2 "does not answer"
run "a missing binary is refused" "$T/nonexistent" 2 "no pinned-server"
run "no argument is refused" "" 2 "no pinned-server"

# The probe runs the binary with an empty environment: no real box, Slack or key reaches it.
stub debug 2 "pinned-server: UPSTASH_BOX_BASE_URL is not set"
sh "$HERE/check-bin.sh" "$T/debug" >/dev/null 2>&1
[ "$(cat "$T/seen-env")" = ":" ] || fail "the probed binary saw UPSTASH_BOX_BASE_URL or PINNED_SLACK_API_BASE"
UPSTASH_BOX_API_KEY=box_real-s3cret ANTHROPIC_API_KEY=sk-s3cret sh "$HERE/check-bin.sh" "$T/debug" >/dev/null 2>&1
grep -q 's3cret' "$T/seen-all" && fail "the probed binary saw a key"
# The probe is a hub start that must refuse before it listens: it names a db that is not there, a port 0.
grep -q -- '--dev' "$T/seen-args" && grep -q -- 'hub' "$T/seen-args" || fail "the probe did not run hub --dev"
[ $failed -eq 0 ] && echo "ok the probed binary got no box, Slack or key in its environment"

if [ $failed -eq 0 ]; then echo "check-bin: all cases pass"; else exit 1; fi
