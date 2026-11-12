#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: scripts/updater-fragment.sh --target TARGET --tag TAG --repo OWNER/REPO --version VERSION --paths JSON --out FILE [--stage DIR]" >&2
  exit 2
}

target=""
tag=""
repo=""
version=""
paths=""
out=""
stage=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) target="${2-}"; shift 2 ;;
    --tag) tag="${2-}"; shift 2 ;;
    --repo) repo="${2-}"; shift 2 ;;
    --version) version="${2-}"; shift 2 ;;
    --paths) paths="${2-}"; shift 2 ;;
    --out) out="${2-}"; shift 2 ;;
    --stage) stage="${2-}"; shift 2 ;;
    *) usage ;;
  esac
done

[[ -n "$target" && -n "$tag" && -n "$repo" && -n "$version" && -n "$paths" && -n "$out" ]] || usage

asset_arch=""
linux_arch=""
case "$target" in
  aarch64-apple-darwin)
    asset_arch="aarch64"
    keys='["darwin-aarch64","darwin-aarch64-app"]'
    kind="app"
    ;;
  x86_64-apple-darwin)
    asset_arch="x64"
    keys='["darwin-x86_64","darwin-x86_64-app"]'
    kind="app"
    ;;
  x86_64-pc-windows-msvc)
    keys='["windows-x86_64","windows-x86_64-nsis"]'
    kind="exe"
    ;;
  x86_64-unknown-linux-gnu)
    linux_arch="x86_64"
    kind="linux"
    ;;
  aarch64-unknown-linux-gnu)
    linux_arch="aarch64"
    kind="linux"
    ;;
  *)
    echo "no updater platforms for $target" >&2
    exit 1
    ;;
esac

sigs=()
appimage_sigs=()
deb_sigs=()
while IFS= read -r path; do
  [[ -n "$path" ]] || continue
  path="${path//\\//}"
  case "$kind" in
    app)
      case "$path" in
        *.app.tar.gz.sig) sigs+=("$path") ;;
      esac
      ;;
    exe)
      case "$path" in
        *-setup.exe.sig) sigs+=("$path") ;;
      esac
      ;;
    linux)
      case "$path" in
        *.AppImage.sig) appimage_sigs+=("$path") ;;
        *.deb.sig) deb_sigs+=("$path") ;;
      esac
      ;;
  esac
done < <(FRAGMENT_PATHS="$paths" node << 'JS'
const paths = JSON.parse(process.env.FRAGMENT_PATHS);
for (const path of paths) console.log(path);
JS
)

if [[ "$kind" == "linux" ]]; then
  if [[ ${#appimage_sigs[@]} -ne 1 ]]; then
    echo "expected one AppImage updater signature for $target, found ${#appimage_sigs[@]}" >&2
    exit 1
  fi
  if [[ ${#deb_sigs[@]} -ne 1 ]]; then
    echo "expected one deb updater signature for $target, found ${#deb_sigs[@]}" >&2
    exit 1
  fi
  sigs=("${appimage_sigs[0]}" "${deb_sigs[0]}")
elif [[ ${#sigs[@]} -ne 1 ]]; then
  echo "expected one updater signature for $target, found ${#sigs[@]}" >&2
  exit 1
fi

for sig in "${sigs[@]}"; do
  [[ -s "$sig" ]] || {
    echo "$sig is missing" >&2
    exit 1
  }
done

entries=""
add_entry() {
  entries+="$1"$'\t'"$2"$'\t'"$3"$'\n'
}

if [[ "$kind" == "linux" ]]; then
  appimage_sig="${sigs[0]}"
  deb_sig="${sigs[1]}"
  add_entry "[\"linux-${linux_arch}\",\"linux-${linux_arch}-appimage\"]" "$appimage_sig" "$(basename "${appimage_sig%.sig}")"
  add_entry "[\"linux-${linux_arch}-deb\"]" "$deb_sig" "$(basename "${deb_sig%.sig}")"
elif [[ "$kind" == "app" ]]; then
  base=$(basename "${sigs[0]}")
  name="${base%.app.tar.gz.sig}"
  add_entry "$keys" "${sigs[0]}" "${name}_${asset_arch}.app.tar.gz"
else
  add_entry "$keys" "${sigs[0]}" "$(basename "${sigs[0]%.sig}")"
fi

if [[ -n "$stage" ]]; then
  mkdir -p "$stage"
  while IFS=$'\t' read -r _ sig asset; do
    [[ -n "$sig" ]] || continue
    [[ -s "${sig%.sig}" ]] || {
      echo "${sig%.sig} is missing" >&2
      exit 1
    }
    cp "${sig%.sig}" "$stage/$asset"
  done <<< "$entries"
fi

FRAGMENT_OUT="$out" \
FRAGMENT_VERSION="$version" \
FRAGMENT_BASE="https://github.com/${repo}/releases/download/${tag}" \
FRAGMENT_ENTRIES="$entries" \
node << 'JS'
const fs = require("fs");
const platforms = {};
for (const line of process.env.FRAGMENT_ENTRIES.split("\n")) {
  if (!line) continue;
  const [keys, sigPath, asset] = line.split("\t");
  const signature = fs.readFileSync(sigPath, "utf8");
  const url = `${process.env.FRAGMENT_BASE}/${asset}`;
  for (const key of JSON.parse(keys)) platforms[key] = { signature, url };
}
fs.writeFileSync(
  process.env.FRAGMENT_OUT,
  JSON.stringify({ version: process.env.FRAGMENT_VERSION, platforms }),
);
JS
