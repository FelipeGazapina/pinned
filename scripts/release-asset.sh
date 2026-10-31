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

draft_release() {
  gh release edit "$tag" --draft=true
  is_draft=$(gh release view "$tag" --json isDraft --jq .isDraft)
  [[ "$is_draft" == "true" ]] || {
    echo "the release $tag is public, so no asset is uploaded" >&2
    exit 1
  }
}

hash_line() {
  local file="$1"
  local digest
  digest=$(sha256sum "$file" | awk '{print $1}')
  printf '%s  %s\n' "$digest" "$(basename "$file")"
}

case "$cmd" in
  draft)
    [[ "$#" -eq 0 ]] || usage
    draft_release
    ;;
  upload)
    [[ "$#" -ge 1 ]] || usage
    seen_sums=0
    latest=""
    sums=""
    for file in "$@"; do
      [[ -s "$file" ]] || {
        echo "$file is missing" >&2
        exit 1
      }
      name=$(basename "$file")
      if [[ "$name" == "SHA256SUMS" ]]; then
        seen_sums=1
        sums=$file
      fi
      if [[ "$name" == "latest.json" ]]; then
        [[ "$seen_sums" -eq 1 ]] || {
          echo "latest.json is ahead of SHA256SUMS" >&2
          exit 1
        }
        latest=$file
      fi
    done
    if [[ -n "$latest" ]]; then
      line=$(hash_line "$latest")
      grep -q -x -F "$line" "$sums" || {
        echo "the release checksum does not match" >&2
        exit 1
      }
    fi
    draft_release
    gh release upload "$tag" "$@" --clobber
    ;;
  publish)
    [[ "$#" -eq 0 ]] || usage
    dir=$(mktemp -d)
    trap 'rm -rf "$dir"' EXIT
    gh release download "$tag" -D "$dir" \
      -p latest.json \
      -p SHA256SUMS \
      -p SHA256SUMS.minisig
    [[ -s "$dir/SHA256SUMS.minisig" ]] || {
      echo "SHA256SUMS.minisig is missing" >&2
      exit 1
    }
    line=$(hash_line "$dir/latest.json")
    grep -q -x -F "$line" "$dir/SHA256SUMS" || {
      echo "the release checksum does not match" >&2
      exit 1
    }
    gh release edit "$tag" --draft=false --latest
    ;;
  *)
    usage
    ;;
esac
