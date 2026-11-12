#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
fragment="$root/scripts/updater-fragment.sh"
assemble="$root/scripts/assemble-latest-json.sh"
notes="On a Mac, install the .dmg. On Windows, run the _x64-setup.exe. On Linux (beta), use the AppImage or the .deb, for x86_64 and arm64. The AppImage needs FUSE, or run it with APPIMAGE_EXTRACT_AND_RUN=1. On Linux, dictation and browser-panel screenshots need macOS. Untested so far on Linux are the tray icon on a real desktop including GNOME, Wayland (it runs through XWayland), notifications, keep awake and the updater. Beta testers, please report problems with those. Or click the update button in a running pinned."
legacy="darwin-aarch64,darwin-aarch64-app,darwin-x86_64,darwin-x86_64-app,windows-x86_64,windows-x86_64-nsis"
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

bash "$assemble" --required "$legacy" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/latest.json" "$work/frags"
bash "$assemble" --required "$legacy" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/latest-again.json" "$work/frags"
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
  bash "$assemble" --required "$legacy" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/partial.json" "$work/no-win"

mkdir -p "$work/clash/a" "$work/clash/b"
cp "$work/frags/updater-aarch64-apple-darwin/updater-fragment.json" "$work/clash/a/updater-fragment.json"
jq '.platforms["darwin-aarch64"].url = "https://example.invalid/other"' \
  "$work/frags/updater-aarch64-apple-darwin/updater-fragment.json" \
  > "$work/clash/b/updater-fragment.json"
expect_fail "darwin-aarch64 was built twice with different bytes" \
  bash "$assemble" --required "$legacy" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/clash.json" "$work/clash"

expect_fail "does not match tag v0.17.0" \
  bash "$assemble" --required "$legacy" --notes "$notes" --pub-date "$pub_date" --tag "v0.17.0" --out "$work/wrong-tag.json" "$work/frags"
expect_fail "does not match tag vv0.16.0" \
  bash "$assemble" --required "$legacy" --notes "$notes" --pub-date "$pub_date" --tag "vv0.16.0" --out "$work/two-v.json" "$work/frags"

bash "$assemble" --required "$legacy" --notes "$notes" --pub-date "$pub_date" --tag "0.16.0" --out "$work/bare-tag.json" "$work/frags"
echo "ok: tag v0.16.0 matches version 0.16.0, and a bare 0.16.0 tag matches too"

# Linux legs: an AppImage and a .deb per architecture, both signed by the updater.
linux_leg() {
  local target="$1" arch="$2" appimage_arch="$3" deb_arch="$4"
  local dir="$work/build/linux-$arch"
  local appimage="pinned_${version}_${appimage_arch}.AppImage"
  local deb="pinned_${version}_${deb_arch}.deb"
  mkdir -p "$dir" "$work/frags-linux/updater-$target"
  printf 'appimage-%s' "$arch" > "$dir/$appimage"
  printf 'deb-%s' "$arch" > "$dir/$deb"
  write_sig "$dir/$appimage.sig" "sig-appimage-$arch"
  write_sig "$dir/$deb.sig" "sig-deb-$arch"
  write_sig "$dir/$appimage.tar.gz.sig" "sig-legacy-$arch"
  one_leg "$target" "$work/frags-linux/updater-$target/updater-fragment.json" \
    "$dir/$appimage" "$dir/$appimage.sig" "$dir/$appimage.tar.gz.sig" "$dir/$deb" "$dir/$deb.sig"
}
linux_leg x86_64-unknown-linux-gnu x86_64 amd64 amd64
linux_leg aarch64-unknown-linux-gnu aarch64 aarch64 arm64

base_url="https://github.com/${repo}/releases/download/${tag}"
jq -e --arg base "$base_url" --arg v "$version" '
  (.platforms | keys_unsorted) == ["linux-x86_64", "linux-x86_64-appimage", "linux-x86_64-deb"]
  and .platforms["linux-x86_64"] == .platforms["linux-x86_64-appimage"]
  and .platforms["linux-x86_64"].signature == "sig-appimage-x86_64"
  and .platforms["linux-x86_64"].url == "\($base)/pinned_\($v)_amd64.AppImage"
  and .platforms["linux-x86_64-deb"].signature == "sig-deb-x86_64"
  and .platforms["linux-x86_64-deb"].url == "\($base)/pinned_\($v)_amd64.deb"
' "$work/frags-linux/updater-x86_64-unknown-linux-gnu/updater-fragment.json" >/dev/null \
  || fail "x86_64 linux fragment"
jq -e --arg base "$base_url" --arg v "$version" '
  .platforms["linux-aarch64"].url == "\($base)/pinned_\($v)_aarch64.AppImage"
  and .platforms["linux-aarch64-appimage"] == .platforms["linux-aarch64"]
  and .platforms["linux-aarch64-deb"].url == "\($base)/pinned_\($v)_arm64.deb"
  and .platforms["linux-aarch64-deb"].signature == "sig-deb-aarch64"
' "$work/frags-linux/updater-aarch64-unknown-linux-gnu/updater-fragment.json" >/dev/null \
  || fail "aarch64 linux fragment"
echo "ok: linux fragments"
echo "    linux-x86_64 $base_url/pinned_${version}_amd64.AppImage"
echo "    linux-x86_64-deb $base_url/pinned_${version}_amd64.deb"
echo "    linux-aarch64 $base_url/pinned_${version}_aarch64.AppImage"
echo "    linux-aarch64-deb $base_url/pinned_${version}_arm64.deb"

x86_dir="$work/build/linux-x86_64"
expect_fail "expected one AppImage updater signature" \
  bash "$fragment" --target x86_64-unknown-linux-gnu --tag "$tag" --repo "$repo" --version "$version" \
  --paths "[\"$x86_dir/pinned_${version}_amd64.deb.sig\"]" --out "$work/no-appimage.json"
expect_fail "expected one deb updater signature" \
  bash "$fragment" --target x86_64-unknown-linux-gnu --tag "$tag" --repo "$repo" --version "$version" \
  --paths "[\"$x86_dir/pinned_${version}_amd64.AppImage.sig\"]" --out "$work/no-deb.json"
expect_fail "expected one AppImage updater signature" \
  bash "$fragment" --target x86_64-unknown-linux-gnu --tag "$tag" --repo "$repo" --version "$version" \
  --paths "[\"$x86_dir/pinned_${version}_amd64.AppImage.sig\",\"$x86_dir/other.AppImage.sig\",\"$x86_dir/pinned_${version}_amd64.deb.sig\"]" \
  --out "$work/two-appimage.json"

stage="$work/stage"
bash "$fragment" --target aarch64-unknown-linux-gnu --tag "$tag" --repo "$repo" --version "$version" \
  --paths "$(jq -n --args '$ARGS.positional' \
    "$work/build/linux-aarch64/pinned_${version}_aarch64.AppImage.sig" "$work/build/linux-aarch64/pinned_${version}_arm64.deb.sig")" \
  --out "$work/staged.json" --stage "$stage"
[[ "$(cat "$stage/pinned_${version}_aarch64.AppImage")" == "appimage-aarch64" ]] || fail "stage lacks the AppImage"
[[ "$(cat "$stage/pinned_${version}_arm64.deb")" == "deb-aarch64" ]] || fail "stage lacks the deb"
echo "ok: --stage copies each bundle under its release asset name"

mkdir -p "$work/all"
cp -R "$work/frags/." "$work/all/"
cp -R "$work/frags-linux/." "$work/all/"
bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/latest-linux.json" "$work/all"
keys=$(jq -r '.platforms | keys_unsorted | join(" ")' "$work/latest-linux.json")
echo "linux fixture platforms: $keys"
want="darwin-aarch64 darwin-aarch64-app darwin-x86_64 darwin-x86_64-app linux-aarch64 linux-aarch64-appimage linux-aarch64-deb linux-x86_64 linux-x86_64-appimage linux-x86_64-deb windows-x86_64 windows-x86_64-nsis"
[[ "$keys" == "$want" ]] || fail "linux platform order: $keys"
bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/latest-linux-again.json" "$work/all"
cmp "$work/latest-linux.json" "$work/latest-linux-again.json"
jq -e '.platforms["linux-x86_64-deb"].url | endswith("_amd64.deb")' "$work/latest-linux.json" >/dev/null
mkdir -p "$work/reversed"
for leg in updater-x86_64-unknown-linux-gnu updater-x86_64-pc-windows-msvc updater-x86_64-apple-darwin updater-aarch64-unknown-linux-gnu updater-aarch64-apple-darwin; do
  mkdir -p "$work/reversed/z-${leg#updater-}"
  cp "$work/all/$leg/updater-fragment.json" "$work/reversed/z-${leg#updater-}/"
done
bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/latest-reversed.json" "$work/reversed"
cmp "$work/latest-linux.json" "$work/latest-reversed.json"
echo "ok: all twelve keys in sorted order whatever the fragment folders are called, deb and appimage urls apart, second assemble matches the first"

mkdir -p "$work/no-arm-linux"
cp -R "$work/all/." "$work/no-arm-linux/"
rm -r "$work/no-arm-linux/updater-aarch64-unknown-linux-gnu"
expect_fail "latest.json is missing linux-aarch64, linux-aarch64-appimage, linux-aarch64-deb" \
  bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/no-arm-linux.json" "$work/no-arm-linux"
mkdir -p "$work/no-x86-linux"
cp -R "$work/all/." "$work/no-x86-linux/"
rm -r "$work/no-x86-linux/updater-x86_64-unknown-linux-gnu"
expect_fail "latest.json is missing linux-x86_64, linux-x86_64-appimage, linux-x86_64-deb" \
  bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/no-x86-linux.json" "$work/no-x86-linux"
expect_fail "latest.json is missing linux-aarch64" \
  bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/legacy-only.json" "$work/frags"
bash "$assemble" --required "$legacy" --notes "$notes" --pub-date "$pub_date" --tag "$tag" --out "$work/legacy-ok.json" "$work/frags"
echo "ok: the default key list needs Linux, --required keeps the legacy six"

dry_tag="v${version}-dryrun"
bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$dry_tag" --version "$version" --out "$work/dry.json" "$work/all"
cmp "$work/latest-linux.json" "$work/dry.json"
expect_fail "does not match tag $dry_tag" \
  bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$dry_tag" --out "$work/dry-no-version.json" "$work/all"
expect_fail "does not match tag $dry_tag" \
  bash "$assemble" --notes "$notes" --pub-date "$pub_date" --tag "$dry_tag" --version "9.9.9" --out "$work/dry-wrong.json" "$work/all"
echo "ok: --version lets a placeholder tag assemble, and a wrong version still fails"

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
bash "$assemble" --required "$legacy" --notes "$real_notes" --pub-date "$real_pub" --tag "v${real_version}" --out "$work/real.json" "$split"
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
