#!/usr/bin/env bash
# Usage: scripts/sign-node-sums.sh <directory>
# <directory>/SHA256SUMS must already exist.
# Tauri writes SHA256SUMS.sig as base64 of a minisign signature.
# The node decodes the raw minisign text, so this writes that text to SHA256SUMS.minisig.
# Needs TAURI_SIGNING_PRIVATE_KEY. An empty password matches the release workflow.
set -euo pipefail

sums_dir=$(cd "${1:?usage: scripts/sign-node-sums.sh <directory>}" && pwd)
repo=$(cd "$(dirname "$0")/.." && pwd)

test -n "${TAURI_SIGNING_PRIVATE_KEY:-}" || {
  echo "TAURI_SIGNING_PRIVATE_KEY is not set" >&2
  exit 1
}
test -f "$sums_dir/SHA256SUMS" || {
  echo "$sums_dir/SHA256SUMS is missing" >&2
  exit 1
}

export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD-}"

npx --prefix "$repo" tauri signer sign "$sums_dir/SHA256SUMS"
openssl base64 -d -A -in "$sums_dir/SHA256SUMS.sig" -out "$sums_dir/SHA256SUMS.minisig"
head -n 1 "$sums_dir/SHA256SUMS.minisig" | grep -q '^untrusted comment:'
