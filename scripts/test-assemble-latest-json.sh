#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
fragment="$root/scripts/updater-fragment.sh"
assemble="$root/scripts/assemble-latest-json.sh"
notes="On a Mac, install the .dmg. On Windows, run the _x64-setup.exe. Or click the update button in a running pinned."
pub_date="2026-10-30T22:10:25.111Z"
repo="FelipeGazapina/pinned"
tag="v0.16.0"
version="0.16.0"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

expect_fail() {
  local needle="$1"
  shift
  local err="$work/stderr"
  if "$@" >"$work/stdout" 2>"$err"; then
    fail "expected failure: $*"
  fi
  if ! grep -q "$needle" "$err"; then
    echo "stderr was:" >&2
    cat "$err" >&2
    fail "stderr missing [$needle]"
  fi
  echo "ok: rejected ($needle)"
  echo "    $(tr '\n' ' ' < "$err")"
}

write_sig() {
  local path="$1"
  local body="$2"
  mkdir -p "$(dirname "$path")"
  printf '%s' "$body" > "$path"
}

one_leg() {
  local target="$1"
  local out="$2"
  shift 2
  local paths_json
  paths_json=$(jq -n --args '$ARGS.positional' "$@")
  bash "$fragment" \
    --target "$target" \
    --tag "$tag" \
    --repo "$repo" \
    --version "$version" \
    --paths "$paths_json" \
    --out "$out"
}

arm="$work/build/aarch64/pinned.app.tar.gz.sig"
intel="$work/build/x64/pinned.app.tar.gz.sig"
win="$work/build/win/pinned_0.16.0_x64-setup.exe.sig"
write_sig "$arm" "sig-arm"
write_sig "$intel" "sig-intel"
write_sig "$win" "sig-win"

mkdir -p "$work/frags/updater-aarch64-apple-darwin" \
  "$work/frags/updater-x86_64-apple-darwin" \
  "$work/frags/updater-x86_64-pc-windows-msvc"

one_leg aarch64-apple-darwin "$work/frags/updater-aarch64-apple-darwin/updater-fragment.json" \
  "$work/build/aarch64/pinned.app" \
  "$work/build/aarch64/pinned.app.tar.gz" \
  "$arm" \
  "$work/build/aarch64/pinned_0.16.0_aarch64.dmg"
one_leg x86_64-apple-darwin "$work/frags/updater-x86_64-apple-darwin/updater-fragment.json" \
  "$intel"
one_leg x86_64-pc-windows-msvc "$work/frags/updater-x86_64-pc-windows-msvc/updater-fragment.json" \
  "$work/build/win/pinned_0.16.0_x64-setup.exe" \
  "$win"

arm_url="https://github.com/${repo}/releases/download/${tag}/pinned_aarch64.app.tar.gz"
intel_url="https://github.com/${repo}/releases/download/${tag}/pinned_x64.app.tar.gz"
win_url="https://github.com/${repo}/releases/download/${tag}/pinned_0.16.0_x64-setup.exe"

jq -e --arg url "$arm_url" '
  .version == "0.16.0"
  and (.platforms["darwin-aarch64"].signature == "sig-arm")
  and (.platforms["darwin-aarch64"].url == $url)
  and (.platforms["darwin-aarch64-app"] == .platforms["darwin-aarch64"])
' "$work/frags/updater-aarch64-apple-darwin/updater-fragment.json" >/dev/null
jq -e --arg url "$intel_url" '
  (.platforms["darwin-x86_64"].signature == "sig-intel")
  and (.platforms["darwin-x86_64"].url == $url)
  and (.platforms["darwin-x86_64-app"] == .platforms["darwin-x86_64"])
' "$work/frags/updater-x86_64-apple-darwin/updater-fragment.json" >/dev/null
jq -e --arg url "$win_url" '
  (.platforms["windows-x86_64"].signature == "sig-win")
  and (.platforms["windows-x86_64"].url == $url)
  and (.platforms["windows-x86_64-nsis"] == .platforms["windows-x86_64"])
' "$work/frags/updater-x86_64-pc-windows-msvc/updater-fragment.json" >/dev/null
bs_json=$(jq -n --arg p "${win//\//\\}" '[$p]')
bash "$fragment" --target x86_64-pc-windows-msvc --tag "$tag" --repo "$repo" --version "$version" \
  --paths "$bs_json" --out "$work/win-bs.json"
jq -e --arg url "$win_url" '.platforms["windows-x86_64"].url == $url and .platforms["windows-x86_64"].signature == "sig-win"' \
  "$work/win-bs.json" >/dev/null
echo "ok: fixture legs"
echo "    darwin-aarch64 $arm_url"
echo "    darwin-x86_64 $intel_url"
echo "    windows-x86_64 $win_url"

expect_fail "expected one updater signature" \
  bash "$fragment" --target aarch64-apple-darwin --tag "$tag" --repo "$repo" --version "$version" \
  --paths "[\"$arm\",\"$intel\"]" --out "$work/two.json"

bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/latest.json" "$work/frags"
bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/latest-again.json" "$work/frags"
cmp "$work/latest.json" "$work/latest-again.json"
echo "ok: second assemble matches the first"

keys=$(jq -r '.platforms | keys_unsorted | join(" ")' "$work/latest.json")
echo "fixture platforms: $keys"
[[ "$keys" == "darwin-aarch64 darwin-aarch64-app darwin-x86_64 darwin-x86_64-app windows-x86_64 windows-x86_64-nsis" ]] \
  || fail "platform order: $keys"
jq -e --arg notes "$notes" --arg pub "$pub_date" '
  .version == "0.16.0" and .notes == $notes and .pub_date == $pub
' "$work/latest.json" >/dev/null
[[ "$(tail -c 1 "$work/latest.json" | wc -l | tr -d ' ')" == "0" ]] || fail "latest.json ends with a newline"

mkdir -p "$work/no-win/updater-aarch64-apple-darwin" "$work/no-win/updater-x86_64-apple-darwin"
cp "$work/frags/updater-aarch64-apple-darwin/updater-fragment.json" "$work/no-win/updater-aarch64-apple-darwin/"
cp "$work/frags/updater-x86_64-apple-darwin/updater-fragment.json" "$work/no-win/updater-x86_64-apple-darwin/"
expect_fail "latest.json is missing windows-x86_64, windows-x86_64-nsis" \
  bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/partial.json" "$work/no-win"

mkdir -p "$work/clash/a" "$work/clash/b"
cp "$work/frags/updater-aarch64-apple-darwin/updater-fragment.json" "$work/clash/a/updater-fragment.json"
jq '.platforms["darwin-aarch64"].url = "https://example.invalid/other"' \
  "$work/frags/updater-aarch64-apple-darwin/updater-fragment.json" \
  > "$work/clash/b/updater-fragment.json"
expect_fail "darwin-aarch64 was built twice with different bytes" \
  bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/clash.json" "$work/clash"

expect_fail "does not match tag v0.17.0" \
  bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "v0.17.0" --out "$work/wrong-tag.json" "$work/frags"
expect_fail "does not match tag vv0.16.0" \
  bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "vv0.16.0" --out "$work/two-v.json" "$work/frags"

bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "0.16.0" --out "$work/bare-tag.json" "$work/frags"
echo "ok: tag v0.16.0 matches version 0.16.0, and a bare 0.16.0 tag matches too"

if [[ $# -eq 0 ]]; then
  echo "skipped real release shape (pass the v0.16.0 download directory)"
  exit 0
fi

release="$1"
[[ -s "$release/latest.json" ]] || fail "missing $release/latest.json"

split="$work/real"
mkdir -p "$split/updater-aarch64-apple-darwin" "$split/updater-x86_64-apple-darwin" "$split/updater-x86_64-pc-windows-msvc"
jq '{version: .version, platforms: {"darwin-aarch64": .platforms["darwin-aarch64"], "darwin-aarch64-app": .platforms["darwin-aarch64-app"]}}' \
  "$release/latest.json" > "$split/updater-aarch64-apple-darwin/updater-fragment.json"
jq '{version: .version, platforms: {"darwin-x86_64": .platforms["darwin-x86_64"], "darwin-x86_64-app": .platforms["darwin-x86_64-app"]}}' \
  "$release/latest.json" > "$split/updater-x86_64-apple-darwin/updater-fragment.json"
jq '{version: .version, platforms: {"windows-x86_64": .platforms["windows-x86_64"], "windows-x86_64-nsis": .platforms["windows-x86_64-nsis"]}}' \
  "$release/latest.json" > "$split/updater-x86_64-pc-windows-msvc/updater-fragment.json"

real_notes=$(jq -r '.notes' "$release/latest.json")
real_pub=$(jq -r '.pub_date' "$release/latest.json")
real_version=$(jq -r '.version' "$release/latest.json")
bash "$assemble" --notes "$real_notes" --pub-date "$real_pub" --tag "v${real_version}" --out "$work/real.json" "$split"
if cmp -s "$release/latest.json" "$work/real.json"; then
  echo "ok: v${real_version} latest.json byte match"
else
  echo "byte diff:" >&2
  diff -u "$release/latest.json" "$work/real.json" >&2 || true
  fail "reassembled latest.json does not match the release bytes"
fi

for triple in aarch64-apple-darwin x86_64-apple-darwin; do
  if [[ "$triple" == "aarch64-apple-darwin" ]]; then
    src="$release/pinned_aarch64.app.tar.gz.sig"
    arch="aarch64"
  else
    src="$release/pinned_x64.app.tar.gz.sig"
    arch="x64"
  fi
  [[ -s "$src" ]] || fail "missing $src"
  leg="$work/sig-$arch"
  mkdir -p "$leg"
  cp "$src" "$leg/pinned.app.tar.gz.sig"
  bash "$fragment" --target "$triple" --tag "v${real_version}" --repo "$repo" --version "$real_version" \
    --paths "[\"$leg/pinned.app.tar.gz.sig\"]" --out "$leg/updater-fragment.json"
done

win_src="$release/pinned_${real_version}_x64-setup.exe.sig"
[[ -s "$win_src" ]] || fail "missing $win_src"
win_leg="$work/sig-win"
mkdir -p "$win_leg"
cp "$win_src" "$win_leg/pinned_${real_version}_x64-setup.exe.sig"
bash "$fragment" --target x86_64-pc-windows-msvc --tag "v${real_version}" --repo "$repo" --version "$real_version" \
  --paths "[\"$win_leg/pinned_${real_version}_x64-setup.exe.sig\"]" --out "$win_leg/updater-fragment.json"

jq -e --slurpfile live "$release/latest.json" '
  . as $frag
  | ($live[0].platforms["darwin-aarch64"] == $frag.platforms["darwin-aarch64"])
' "$work/sig-aarch64/updater-fragment.json" >/dev/null \
  || fail "aarch64 fragment does not match the release"
jq -e --slurpfile live "$release/latest.json" '
  .platforms["darwin-x86_64"] == $live[0].platforms["darwin-x86_64"]
' "$work/sig-x64/updater-fragment.json" >/dev/null \
  || fail "x64 fragment does not match the release"
jq -e --slurpfile live "$release/latest.json" '
  .platforms["windows-x86_64"] == $live[0].platforms["windows-x86_64"]
' "$win_leg/updater-fragment.json" >/dev/null \
  || fail "windows fragment does not match the release"
echo "ok: updater fragments match v${real_version} signature and url"
