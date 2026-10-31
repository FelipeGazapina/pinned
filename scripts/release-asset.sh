#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: scripts/release-asset.sh draft|upload|publish <tag> [files...]" >&2
  exit 2
}

cmd="${1:-}"
tag="${2:-}"
[[ -n "$cmd" && -n "$tag" ]] || usage
shift 2
repo=$(cd "$(dirname "$0")/.." && pwd)

refuse() {
  echo "$1" >&2
  exit 1
}

draft_release() {
  gh release edit "$tag" --draft=true
  is_draft=$(gh release view "$tag" --json isDraft --jq .isDraft)
  [[ "$is_draft" == "true" ]] || refuse "the release $tag is public, so no asset is uploaded"
}

hash_of() {
  sha256sum "$1" | awk '{print $1}'
}

present() {
  [[ -f "$1/$2" ]] || refuse "$2 is missing from the release"
}

version_is_newer() {
  local left="${1#v}"
  local right="${2#v}"
  local IFS=.
  local -a left_parts right_parts
  local i left_n right_n
  read -r -a left_parts <<< "$left"
  read -r -a right_parts <<< "$right"
  for i in 0 1 2; do
    left_n="${left_parts[$i]:-0}"
    right_n="${right_parts[$i]:-0}"
    left_n="${left_n%%[^0-9]*}"
    right_n="${right_n%%[^0-9]*}"
    left_n="${left_n:-0}"
    right_n="${right_n:-0}"
    if ((10#$left_n > 10#$right_n)); then
      return 0
    fi
    if ((10#$left_n < 10#$right_n)); then
      return 1
    fi
  done
  return 1
}

tag_is_latest() {
  local current
  current=$(gh release view --json tagName --jq .tagName 2>/dev/null) || return 0
  ! version_is_newer "$current" "$1"
}

case "$cmd" in
  draft)
    [[ "$#" -eq 0 ]] || usage
    draft_release
    ;;
  upload)
    [[ "$#" -ge 1 ]] || usage
    latest=""
    sums=""
    for file in "$@"; do
      [[ -s "$file" ]] || refuse "$file is missing"
      case "$(basename "$file")" in
        SHA256SUMS) sums=$file ;;
        latest.json) latest=$file ;;
      esac
    done
    if [[ -n "$latest" ]]; then
      grep -q -x -F "$(hash_of "$latest")  latest.json" "$sums" || refuse "the release checksum does not match"
    fi
    draft_release
    gh release upload "$tag" "$@" --clobber
    ;;
  publish)
    [[ "$#" -eq 0 ]] || usage
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    assets="$work/assets"
    gh release download "$tag" -D "$assets"
    pubkey="${PINNED_RELEASE_PUBKEY:-$(jq -r .plugins.updater.pubkey "$repo/tauri.conf.json")}"
    verifier="$repo/scripts/minisign-verify.mjs"
    present "$assets" SHA256SUMS
    present "$assets" SHA256SUMS.minisig
    node "$verifier" "$pubkey" "$assets/SHA256SUMS" "$assets/SHA256SUMS.minisig" ||
      refuse "SHA256SUMS.minisig does not verify SHA256SUMS"
    listed=" "
    while read -r digest name _; do
      name="${name#\*}"
      present "$assets" "$name"
      [[ "$(hash_of "$assets/$name")" == "$digest" ]] || refuse "$name does not match SHA256SUMS"
      listed+="$name "
    done < "$assets/SHA256SUMS"
    for name in latest.json pinned-server-aarch64-unknown-linux-musl pinned-server-x86_64-unknown-linux-musl; do
      [[ "$listed" == *" $name "* ]] || refuse "SHA256SUMS does not list $name"
    done
    bundles=$(jq -r '[.platforms[] | [(.url | split("/") | last), .signature]] | unique[] | @tsv' "$assets/latest.json")
    while IFS=$'\t' read -r name signature; do
      present "$assets" "$name"
      printf '%s' "$signature" | openssl base64 -d -A > "$work/signature"
      node "$verifier" "$pubkey" "$assets/$name" "$work/signature" ||
        refuse "$name does not match its signature in latest.json"
    done <<< "$bundles"
    latest=false
    if tag_is_latest "$tag"; then
      latest=true
    fi
    gh release edit "$tag" --draft=false --latest="$latest"
    ;;
  *)
    usage
    ;;
esac
