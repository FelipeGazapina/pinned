#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: scripts/assemble-latest-json.sh --notes NOTES --pub-date ISO --tag TAG --out FILE [--version VERSION] [--required KEY,KEY,...] DIR" >&2
  exit 2
}

notes=""
pub_date=""
tag=""
out=""
version=""
required_keys=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --notes) notes="${2-}"; shift 2 ;;
    --pub-date) pub_date="${2-}"; shift 2 ;;
    --tag) tag="${2-}"; shift 2 ;;
    --out) out="${2-}"; shift 2 ;;
    --version) version="${2-}"; shift 2 ;;
    --required) required_keys="${2-}"; shift 2 ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done

dir="${1-}"
[[ -n "$notes" && -n "$pub_date" && -n "$tag" && -n "$out" && -n "$dir" ]] || usage
[[ -d "$dir" ]] || {
  echo "$dir is not a directory" >&2
  exit 1
}

if [[ -z "$required_keys" ]]; then
  required_keys="darwin-aarch64,darwin-aarch64-app,darwin-x86_64,darwin-x86_64-app,linux-aarch64,linux-aarch64-appimage,linux-aarch64-deb,linux-x86_64,linux-x86_64-appimage,linux-x86_64-deb,windows-x86_64,windows-x86_64-nsis"
fi
required=$(jq -c -n --arg keys "$required_keys" '$keys | split(",")')

files=()
while IFS= read -r fragment; do
  [[ -n "$fragment" ]] || continue
  files+=("$fragment")
done < <(find "$dir" -name updater-fragment.json -type f | sort)

if [[ ${#files[@]} -eq 0 ]]; then
  echo "no updater fragments in $dir" >&2
  exit 1
fi

merged=$(mktemp)
trap 'rm -f "$merged"' EXIT

if ! jq -s \
  --argjson required "$required" \
  --arg tag "$tag" \
  --arg expected "$version" \
  '
    if any(.[]; (.platforms | type) != "object") then
      error("updater fragment has no platforms")
    elif any(.[]; (.version | type) != "string" or (.version | length) == 0) then
      error("updater fragments disagree on version")
    else . end
    | (map(.version) | unique) as $versions
    | if ($versions | length) != 1 then
        error("updater fragments disagree on version")
      else . end
    | ([ .[] | .platforms | to_entries[] ]) as $entries
    | (reduce $entries[] as $e ({};
        if ($e.value.signature | type) != "string" or ($e.value.signature | length) == 0
          or ($e.value.url | type) != "string" or ($e.value.url | length) == 0 then
          error("platform \($e.key) has no signature or url")
        elif .[$e.key] != null and .[$e.key] != $e.value then
          error("platform \($e.key) was built twice with different bytes")
        else
          .[$e.key] = $e.value
        end
      )) as $platforms
    | ($required - ($platforms | keys)) as $missing
    | if ($missing | length) > 0 then
        error("latest.json is missing \($missing | join(", "))")
      else . end
    | $versions[0] as $version
    | (if $expected != "" then $expected else ($tag | if startswith("v") then .[1:] else . end) end) as $bare
    | if $version != $bare then
        error("latest.json version \($version) does not match tag \($tag)")
      else
        {version: $version, platforms: ($platforms | to_entries | sort_by(.key) | from_entries)}
      end
  ' "${files[@]}" > "$merged"; then
  exit 1
fi

dest=$(mktemp)
trap 'rm -f "$merged" "$dest"' EXIT
jq -j -n \
  --arg notes "$notes" \
  --arg pub_date "$pub_date" \
  --slurpfile merged "$merged" \
  '{version: $merged[0].version, notes: $notes, pub_date: $pub_date, platforms: $merged[0].platforms}' \
  > "$dest"
mv "$dest" "$out"
