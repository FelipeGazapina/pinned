#!/usr/bin/env bash
# Prints, in $GITHUB_ENV syntax, the signing and notarization variables tauri-action reads, for the
# secrets that exist. Without APPLE_CERTIFICATE it prints nothing and the release stays ad-hoc
# signed: tauri reads a variable that is set but empty as a certificate or identity, and fails.
# With them it also prints PINNED_DEVELOPER_ID_BUILD=1, which turns the Keychain on.
# docs/release/developer-id.md lists the secrets.
set -euo pipefail

if [ "${RUNNER_OS:-macOS}" != macOS ]; then
  exit 0
fi
if [ -z "${APPLE_CERTIFICATE:-}" ]; then
  echo "apple-signing-env: no APPLE_CERTIFICATE, so this build stays ad-hoc signed" >&2
  exit 0
fi

need() {
  for name in "$@"; do
    if [ -z "${!name:-}" ]; then
      echo "apple-signing-env: $name is missing; docs/release/developer-id.md lists what signing needs" >&2
      exit 1
    fi
  done
}

emit() {
  for name in "$@"; do
    local delimiter="EOF_$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
    printf '%s<<%s\n%s\n%s\n' "$name" "$delimiter" "${!name}" "$delimiter"
  done
}

need APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY
emit APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY
# A compile-time flag: login_store::system() uses the Keychain only in a build signed with a
# Developer ID, because an ad-hoc build would ask for the Mac password after every update.
PINNED_DEVELOPER_ID_BUILD=1
emit PINNED_DEVELOPER_ID_BUILD

if [ -n "${APPLE_API_KEY:-}" ]; then
  need APPLE_API_ISSUER APPLE_API_PRIVATE_KEY
  APPLE_API_KEY_PATH="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/AuthKey_${APPLE_API_KEY}.p8"
  (umask 077 && printf '%s\n' "$APPLE_API_PRIVATE_KEY" > "$APPLE_API_KEY_PATH")
  emit APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
elif [ -n "${APPLE_ID:-}" ]; then
  need APPLE_PASSWORD APPLE_TEAM_ID
  emit APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
else
  echo "apple-signing-env: signing without notarization; set APPLE_API_KEY or APPLE_ID to notarize" >&2
fi
