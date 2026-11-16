#!/bin/sh
# Plain sh: dev.sh must refuse (exit 2, create nothing) a TEAMS_DEV_DIR that is or is inside a real
# pinned home. HOME is a fresh fake home under the work dir, never the real one, and TEAMS_DEV_BIN is
# /bin/false so a guard that was missed fails fast instead of starting anything.
#
#   sh scripts/teams-dev/dev-guard.test.sh
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
WORK=${GUARD_TEST_DIR:-/home/boxuser/work}
[ -d "$WORK" ] || WORK=${TMPDIR:-/tmp}
FAKEHOME=$(mktemp -d "$WORK/guard-home.XXXXXX") || exit 1
trap 'rm -rf "$FAKEHOME"' EXIT
RECORD=$REPO/.dev/teams-dev.dir
failed=0

record_state() { if [ -e "$RECORD" ]; then cat "$RECORD"; else echo "(none)"; fi; }
fail() { echo "FAIL $1" >&2; failed=1; }

# expect_refused <what> <dir> [command]: dev.sh <command> with TEAMS_DEV_DIR=<dir> exits 2 and creates nothing.
expect_refused() {
  what=$1 dir=$2 cmd=${3:-up}
  before=$(record_state)
  # A dir that was already there (/, the fake home, a real /workspace/home) must only be left as it was.
  [ -e "$dir" ] && existed=1 || existed=
  out=$(HOME="$FAKEHOME" TEAMS_DEV_DIR="$dir" TEAMS_DEV_BIN=/bin/false TMPDIR="$FAKEHOME/tmp-never" sh "$HERE/dev.sh" "$cmd" 2>&1)
  code=$?
  [ "$code" -eq 2 ] || fail "$what ($cmd): exit $code, wanted 2: $out"
  [ -n "$existed" ] || [ ! -e "$dir" ] || fail "$what ($cmd): the dir exists afterwards"
  [ "$(record_state)" = "$before" ] || fail "$what ($cmd): .dev/teams-dev.dir was written"
  case $out in *"$dir"*) fail "$what ($cmd): the message names the path" ;; esac
  [ $failed -eq 0 ] && echo "ok $what ($cmd): $out"
}

for cmd in up reset seed; do
  expect_refused "pinned data folder" "$FAKEHOME/.pinned/x" $cmd
  expect_refused "pinned config folder" "$FAKEHOME/.config/pinned" $cmd
  expect_refused "desktop app data folder" "$FAKEHOME/Library/Application Support/dev.personal-harness.pinned/s" $cmd
done
expect_refused "the pinned data folder itself" "$FAKEHOME/.pinned"
expect_refused "a box's persisted home" /workspace/home
expect_refused "inside a box's persisted home" /workspace/home/stack
expect_refused "the home folder" "$FAKEHOME"
expect_refused "the filesystem root" /
# A symlink into a pinned home does not hide it.
mkdir -p "$FAKEHOME/.pinned" && ln -s "$FAKEHOME/.pinned" "$FAKEHOME/innocent"
expect_refused "a symlink to a pinned data folder" "$FAKEHOME/innocent/x"
# A relative path is resolved before it is judged.
mkdir -p "$FAKEHOME/cwd"
before=$(record_state)
out=$(cd "$FAKEHOME/cwd" && HOME="$FAKEHOME" TEAMS_DEV_DIR=../.pinned/x TEAMS_DEV_BIN=/bin/false sh "$HERE/dev.sh" up 2>&1); code=$?
[ "$code" -eq 2 ] || fail "a relative path into a pinned data folder: exit $code: $out"
[ ! -e "$FAKEHOME/.pinned/x" ] || fail "a relative path into a pinned data folder created it"
[ "$(record_state)" = "$before" ] || fail "a relative path: .dev/teams-dev.dir was written"
# A folder that is not an empty dir or a stack's is never emptied.
mkdir -p "$FAKEHOME/notes" && echo keep >"$FAKEHOME/notes/a.txt"
out=$(HOME="$FAKEHOME" TEAMS_DEV_DIR="$FAKEHOME/notes" TEAMS_DEV_BIN=/bin/false sh "$HERE/dev.sh" up 2>&1); code=$?
[ "$code" -eq 2 ] && [ -f "$FAKEHOME/notes/a.txt" ] || fail "a folder with other files was emptied or not refused: exit $code: $out"

# A binary that is not a debug build is refused before the stack touches anything: no marker, no
# record, no database, no port. (A stub that answers like a release build.)
printf '#!/bin/sh\necho "pinned-server: this is a release build: --dev needs a debug build (cargo build -p pinned-server, without --release)" >&2\nexit 2\n' >"$FAKEHOME/release-bin"
chmod +x "$FAKEHOME/release-bin"
mkdir -p "$FAKEHOME/stack-release"
before=$(record_state)
out=$(HOME="$FAKEHOME" TEAMS_DEV_DIR="$FAKEHOME/stack-release" TEAMS_DEV_BIN="$FAKEHOME/release-bin" sh "$HERE/dev.sh" up 2>&1); code=$?
[ "$code" -eq 2 ] || fail "a release binary: exit $code, wanted 2: $out"
case $out in *"not a debug build"*) ;; *) fail "a release binary: the message does not say it is not a debug build: $out" ;; esac
[ -z "$(ls -A "$FAKEHOME/stack-release")" ] || fail "a release binary: the stack dir was written to: $(ls -A "$FAKEHOME/stack-release")"
[ "$(record_state)" = "$before" ] || fail "a release binary: .dev/teams-dev.dir was written"
[ $failed -eq 0 ] && echo "ok a release binary is refused before anything is created: $out"

if [ $failed -eq 0 ]; then echo "dev-guard: all refused, nothing created"; else exit 1; fi
