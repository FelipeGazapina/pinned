#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: scripts/updater-fragment.sh --target TARGET --tag TAG --repo OWNER/REPO --version VERSION --paths JSON --out FILE" >&2
  exit 2
}

target=""
tag=""
repo=""
version=""
paths=""
out=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) target="${2-}"; shift 2 ;;
    --tag) tag="${2-}"; shift 2 ;;
    --repo) repo="${2-}"; shift 2 ;;
    --version) version="${2-}"; shift 2 ;;
    --paths) paths="${2-}"; shift 2 ;;
    --out) out="${2-}"; shift 2 ;;
    *) usage ;;
  esac
done

[[ -n "$target" && -n "$tag" && -n "$repo" && -n "$version" && -n "$paths" && -n "$out" ]] || usage

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
  *)
    echo "no updater platforms for $target" >&2
    exit 1
    ;;
esac

sigs=()
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
  esac
done < <(FRAGMENT_PATHS="$paths" node << 'JS'
const paths = JSON.parse(process.env.FRAGMENT_PATHS);
for (const path of paths) console.log(path);
JS
)

if [[ ${#sigs[@]} -ne 1 ]]; then
  echo "expected one updater signature for $target, found ${#sigs[@]}" >&2
  exit 1
fi

sig="${sigs[0]}"
[[ -s "$sig" ]] || {
  echo "$sig is missing" >&2
  exit 1
}

base=$(basename "$sig")
if [[ "$kind" == "app" ]]; then
  name="${base%.app.tar.gz.sig}"
  asset="${name}_${asset_arch}.app.tar.gz"
else
  asset="${base%.sig}"
fi

url="https://github.com/${repo}/releases/download/${tag}/${asset}"

FRAGMENT_OUT="$out" \
FRAGMENT_VERSION="$version" \
FRAGMENT_URL="$url" \
FRAGMENT_KEYS="$keys" \
FRAGMENT_SIG="$sig" \
node << 'JS'
const fs = require("fs");
const signature = fs.readFileSync(process.env.FRAGMENT_SIG, "utf8");
const keys = JSON.parse(process.env.FRAGMENT_KEYS);
const platforms = {};
for (const key of keys) {
  platforms[key] = { signature, url: process.env.FRAGMENT_URL };
}
fs.writeFileSync(
  process.env.FRAGMENT_OUT,
  JSON.stringify({ version: process.env.FRAGMENT_VERSION, platforms }),
);
JS
