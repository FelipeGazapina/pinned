#!/usr/bin/env bash
# Checks that scripts/apple-signing-env.sh exports signing only when its secrets exist, so a
# release without them still builds ad-hoc signed.
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failures=0

run() {
  env -i PATH="$PATH" RUNNER_TEMP="$tmp" "$@" bash scripts/apple-signing-env.sh 2>"$tmp/stderr"
}

# Turns the $GITHUB_ENV heredoc entries into NAME=value lines.
parse() {
  local name="" delimiter="" line
  while IFS= read -r line; do
    if [ -z "$name" ]; then
      name="${line%%<<*}"
      delimiter="${line#*<<}"
    elif [ "$line" = "$delimiter" ]; then
      name=""
    else
      echo "$name=$line"
    fi
  done
}

expect() {
  if [ "$2" = "$3" ]; then
    echo "ok: $1"
  else
    printf 'FAIL: %s\n  expected: %s\n  got:      %s\n' "$1" "$3" "$2"
    failures=$((failures + 1))
  fi
}

expect "no secrets export nothing" "$(run)" ""
expect "secrets GitHub passes as empty strings export nothing" \
  "$(run APPLE_CERTIFICATE= APPLE_CERTIFICATE_PASSWORD= APPLE_SIGNING_IDENTITY= APPLE_API_KEY= APPLE_ID=)" ""

status=0
out=$(run APPLE_CERTIFICATE=Y2VydA== APPLE_SIGNING_IDENTITY="Developer ID Application: Ana (TEAM123456)") || status=$?
expect "a certificate without its password fails the build" "$status:$out" "1:"
expect "and names what is missing" "$(cat "$tmp/stderr")" \
  "apple-signing-env: APPLE_CERTIFICATE_PASSWORD is missing; docs/release/developer-id.md lists what signing needs"

signing=(APPLE_CERTIFICATE=Y2VydA== APPLE_CERTIFICATE_PASSWORD=p12-pass "APPLE_SIGNING_IDENTITY=Developer ID Application: Ana (TEAM123456)")

expect "a certificate alone signs without notarizing, and marks the build for the Keychain" "$(run "${signing[@]}" | parse)" \
  "$(printf '%s\n' APPLE_CERTIFICATE=Y2VydA== APPLE_CERTIFICATE_PASSWORD=p12-pass "APPLE_SIGNING_IDENTITY=Developer ID Application: Ana (TEAM123456)" PINNED_DEVELOPER_ID_BUILD=1)"

expect "a Developer ID build prints the flag that turns the Keychain on" \
  "$(run "${signing[@]}" | parse | grep -c '^PINNED_DEVELOPER_ID_BUILD=1$')" "1"
expect "an ad-hoc build prints no flag" "$(run | grep -c PINNED_DEVELOPER_ID_BUILD || true)" "0"
expect "empty secrets print no flag" \
  "$(run APPLE_CERTIFICATE= APPLE_CERTIFICATE_PASSWORD= APPLE_SIGNING_IDENTITY= | grep -c PINNED_DEVELOPER_ID_BUILD || true)" "0"

expect "an API key notarizes through a .p8 file" \
  "$(run "${signing[@]}" APPLE_API_KEY=KEY1234567 APPLE_API_ISSUER=issuer-uuid APPLE_API_PRIVATE_KEY=p8-body APPLE_ID=ignored@acme.dev | parse | tail -3)" \
  "$(printf '%s\n' APPLE_API_KEY=KEY1234567 APPLE_API_ISSUER=issuer-uuid "APPLE_API_KEY_PATH=$tmp/AuthKey_KEY1234567.p8")"
expect "the .p8 holds the key" "$(cat "$tmp/AuthKey_KEY1234567.p8")" "p8-body"
expect "readable by the runner's user only" "$(ls -l "$tmp/AuthKey_KEY1234567.p8" | cut -c1-10)" "-rw-------"

expect "an Apple ID notarizes with its app-specific password" \
  "$(run "${signing[@]}" APPLE_ID=ana@acme.dev APPLE_PASSWORD=abcd-efgh APPLE_TEAM_ID=TEAM123456 | parse | tail -3)" \
  "$(printf '%s\n' APPLE_ID=ana@acme.dev APPLE_PASSWORD=abcd-efgh APPLE_TEAM_ID=TEAM123456)"

expect "a Windows leg exports nothing" "$(run RUNNER_OS=Windows "${signing[@]}" APPLE_ID=ana@acme.dev APPLE_PASSWORD=abcd-efgh APPLE_TEAM_ID=TEAM123456)" ""

status=0
run "${signing[@]}" APPLE_ID=ana@acme.dev APPLE_PASSWORD=abcd-efgh >/dev/null || status=$?
expect "an Apple ID without its team fails" "$status" "1"

[ "$failures" -eq 0 ] || { echo "$failures failed"; exit 1; }
echo "all apple-signing-env checks passed"
