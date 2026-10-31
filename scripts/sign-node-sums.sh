#!/usr/bin/env bash
# Tauri writes SHA256SUMS.sig as base64 of a minisign signature.
# The signature algorithm must be ED. Ed is the legacy algorithm, and the node rejects it.
set -euo pipefail

sums_dir=$(cd "${1:?usage: scripts/sign-node-sums.sh <directory>}" && pwd)
repo=$(cd "$(dirname "$0")/.." && pwd)

test -n "${TAURI_SIGNING_PRIVATE_KEY:-}" || {
  echo "TAURI_SIGNING_PRIVATE_KEY is not set" >&2
  exit 1
}

files=(
  latest.json
  pinned-server-aarch64-unknown-linux-musl
  pinned-server-x86_64-unknown-linux-musl
)
for file in "${files[@]}"; do
  test -s "$sums_dir/$file" || {
    echo "$sums_dir/$file is missing" >&2
    exit 1
  }
done

(
  cd "$sums_dir"
  sha256sum "${files[@]}" > SHA256SUMS
)
for file in "${files[@]}"; do
  grep -E -q "^[[:xdigit:]]{64}  ${file}$" "$sums_dir/SHA256SUMS" || {
    echo "SHA256SUMS is missing a line for $file" >&2
    exit 1
  }
done
test "$(wc -l < "$sums_dir/SHA256SUMS")" -eq "${#files[@]}"

export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD-}"

npx --prefix "$repo" tauri signer sign "$sums_dir/SHA256SUMS"
openssl base64 -d -A -in "$sums_dir/SHA256SUMS.sig" -out "$sums_dir/SHA256SUMS.minisig"
head -n 1 "$sums_dir/SHA256SUMS.minisig" | grep -q '^untrusted comment:'

alg=$(sed -n '2p' "$sums_dir/SHA256SUMS.minisig" | openssl base64 -d -A | dd bs=1 count=2 2>/dev/null || true)
test "$alg" = "ED" || {
  echo "the signature algorithm is ${alg:-empty}, not the prehashed ED algorithm" >&2
  exit 1
}
