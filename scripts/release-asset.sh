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
  [[ "$is_draft" == "true" ]] || refuse "gh release edit --draft=true left $tag public, so no asset is uploaded"
}

hash_of() {
  sha256sum "$1" | awk '{print $1}'
}

present() {
  [[ -f "$1/$2" ]] || refuse "$2 is missing from the release"
}

version_is_newer() {
  local LC_ALL=C
  local left="${1#v}" right="${2#v}"
  left="${left%%+*}"
  right="${right%%+*}"
  local -a left_parts right_parts
  local i a b
  IFS=. read -r -a left_parts <<< "${left%%-*}"
  IFS=. read -r -a right_parts <<< "${right%%-*}"
  for i in 0 1 2; do
    a="${left_parts[$i]:-0}"
    b="${right_parts[$i]:-0}"
    a="${a%%[^0-9]*}"
    b="${b%%[^0-9]*}"
    if ((10#${a:-0} > 10#${b:-0})); then return 0; fi
    if ((10#${a:-0} < 10#${b:-0})); then return 1; fi
  done
  [[ "$right" == *-* ]] || return 1
  [[ "$left" == *-* ]] || return 0
  IFS=. read -r -a left_parts <<< "${left#*-}"
  IFS=. read -r -a right_parts <<< "${right#*-}"
  for ((i = 0; i < ${#left_parts[@]}; i++)); do
    ((i < ${#right_parts[@]})) || return 0
    a="${left_parts[$i]}"
    b="${right_parts[$i]}"
    if [[ "$a" == "$b" ]]; then continue; fi
    if [[ "$a" =~ ^[0-9]+$ && "$b" =~ ^[0-9]+$ ]]; then
      if ((10#$a > 10#$b)); then return 0; fi
      return 1
    fi
    if [[ "$a" =~ ^[0-9]+$ ]]; then return 1; fi
    if [[ "$b" =~ ^[0-9]+$ ]]; then return 0; fi
    if [[ "$a" > "$b" ]]; then return 0; fi
    return 1
  done
  return 1
}

tag_is_latest() {
  local current
  if ! current=$(gh release view --json tagName --jq .tagName 2> "$work/latest-error"); then
    grep -q -x -F "release not found" "$work/latest-error" ||
      refuse "could not read the latest release: $(cat "$work/latest-error")"
    return 0
  fi
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
      [[ -n "$sums" ]] || refuse "SHA256SUMS is missing from the upload"
      grep -q -x -F "$(hash_of "$latest")  latest.json" "$sums" || refuse "SHA256SUMS does not list this latest.json"
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
    pubkey=$(jq -r .plugins.updater.pubkey "$repo/tauri.conf.json")
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
    if [[ "$tag" == *-* ]]; then
      flags=(--prerelease --latest=false)
    elif tag_is_latest "$tag"; then
      flags=(--latest=true)
    else
      flags=(--latest=false)
    fi
    gh release edit "$tag" --draft=false "${flags[@]}"
    ;;
  *)
    usage
    ;;
esac
