#!/usr/bin/env bash
# The one release script. The `release` workflow (.github/workflows/release.yml) runs every command here;
# nobody releases from a local checkout. Start a release from Actions > release > Run workflow, on main,
# with the version and dry_run off.
#
#   prepare <version>             bump Cargo.toml and Cargo.lock, write the wire snapshot, commit "Release v<version>", tag
#   fragment ...                  write one desktop leg's updater fragment
#   assemble-latest-json ...      merge the fragments into latest.json
#   sign-sums <dir>               write SHA256SUMS for the node files and sign it
#   apple-env                     print the Developer ID signing variables, in $GITHUB_ENV syntax
#   asset draft|upload|publish <tag> [files...] | verify <dir>
#                                 move the GitHub release between draft and public, checking its signatures
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)

refuse() {
  echo "$1" >&2
  exit 1
}

usage() {
  echo "usage: scripts/release.sh prepare|fragment|assemble-latest-json|sign-sums|apple-env|asset ..." >&2
  exit 2
}

# Needs a clean checkout of main at origin/main, as the workflow makes. It commits and tags locally and pushes nothing:
# the workflow pushes main and the tag in one atomic push, so origin gets both or neither.
cmd_prepare() (
  version="${1:?usage: scripts/release.sh prepare <version>}"
  cd "$repo"

  semver='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
  [[ "$version" =~ $semver ]] || refuse "the version is MAJOR.MINOR.PATCH with an optional -suffix, not $version"
  branch=$(git symbolic-ref --short -q HEAD || true)
  [[ "$branch" == main ]] || refuse "run a release from main, not ${branch:-a detached HEAD}"
  snapshot="crates/core/wire-contract/$version.json"
  [[ -z "$(git rev-parse -q --verify "refs/tags/v$version")" ]] || refuse "a local tag v$version exists. Pick another version."
  [[ ! -e "$snapshot" ]] || refuse "$snapshot exists, so v$version already shipped. Pick another version."
  git fetch -q origin main
  [[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || refuse "HEAD is not origin/main. Run the release from the current main."
  remote_tag=$(git ls-remote origin "refs/tags/v$version")
  [[ -z "$remote_tag" ]] || refuse "origin already has v$version. Pick another version."
  [[ -z "$(git status --porcelain)" ]] || refuse "the tree has uncommitted changes"

  git show HEAD:Cargo.toml | sed "s/^version = \".*\"/version = \"$version\"/" > Cargo.toml
  cargo update -p pinned --quiet
  if [[ "$version" == *-* ]]; then
    echo "v$version is a release candidate. Hub deploy skips it, so it gets no wire snapshot."
  else
    cargo run -q -p pinned-server --bin wire-contract -- snapshot "$version"
    git add "$snapshot"
  fi
  stray=$(git status --porcelain -- ':!Cargo.toml' ':!Cargo.lock' ":!$snapshot")
  [[ -z "$stray" ]] || refuse "the release stopped before the commit, because these files changed while the release built:
  $stray"
  git add Cargo.toml Cargo.lock
  git commit -q -m "Release v$version"
  git tag "v$version"
  echo "Committed and tagged v$version at $(git rev-parse --short HEAD). Push main and the tag together: git push --atomic origin HEAD:refs/heads/main refs/tags/v$version"
)

cmd_fragment() (
  usage() {
    echo "usage: scripts/release.sh fragment --target TARGET --tag TAG --repo OWNER/REPO --version VERSION --paths JSON --out FILE [--stage DIR]" >&2
    exit 2
  }

  target=""
  tag=""
  slug=""
  version=""
  paths=""
  out=""
  stage=""

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --target) target="${2-}"; shift 2 ;;
      --tag) tag="${2-}"; shift 2 ;;
      --repo) slug="${2-}"; shift 2 ;;
      --version) version="${2-}"; shift 2 ;;
      --paths) paths="${2-}"; shift 2 ;;
      --out) out="${2-}"; shift 2 ;;
      --stage) stage="${2-}"; shift 2 ;;
      *) usage ;;
    esac
  done

  [[ -n "$target" && -n "$tag" && -n "$slug" && -n "$version" && -n "$paths" && -n "$out" ]] || usage

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
  FRAGMENT_BASE="https://github.com/${slug}/releases/download/${tag}" \
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
)

cmd_assemble_latest_json() (
  usage() {
    echo "usage: scripts/release.sh assemble-latest-json --notes NOTES --pub-date ISO --tag TAG --out FILE [--version VERSION] [--required KEY,KEY,...] DIR" >&2
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
)

cmd_sign_sums() (
  # Tauri writes SHA256SUMS.sig as base64 of a minisign signature.
  # The signature algorithm must be ED. Ed is the legacy algorithm, and the node rejects it.
  sums_dir=$(cd "${1:?usage: scripts/release.sh sign-sums <directory>}" && pwd)

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
)

cmd_apple_env() (
  # Prints, in $GITHUB_ENV syntax, the signing and notarization variables tauri-action reads, for the
  # secrets that exist. Without APPLE_CERTIFICATE it prints nothing and the release stays ad-hoc
  # signed: tauri reads a variable that is set but empty as a certificate or identity, and fails.
  # With them it also prints PINNED_DEVELOPER_ID_BUILD=1, which turns the Keychain on.
  if [ "${RUNNER_OS:-macOS}" != macOS ]; then
    exit 0
  fi
  if [ -z "${APPLE_CERTIFICATE:-}" ]; then
    echo "apple-signing-env: no APPLE_CERTIFICATE, so this build stays ad-hoc signed" >&2
    exit 0
  fi

  need() {
    for name in "$@"; do
      if [ -z "${!name:-}" ]; then
        echo "apple-signing-env: $name is missing" >&2
        exit 1
      fi
    done
  }

  emit() {
    for name in "$@"; do
      local delimiter
      delimiter="EOF_$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
      printf '%s<<%s\n%s\n%s\n' "$name" "$delimiter" "${!name}" "$delimiter"
    done
  }

  need APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY
  emit APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY
  # A compile-time flag: login_store::system() uses the Keychain only in a build signed with a
  # Developer ID, because an ad-hoc build would ask for the Mac password after every update.
  # shellcheck disable=SC2034 # read by emit through ${!name}
  PINNED_DEVELOPER_ID_BUILD=1
  emit PINNED_DEVELOPER_ID_BUILD

  if [ -n "${APPLE_API_KEY:-}" ]; then
    need APPLE_API_ISSUER APPLE_API_PRIVATE_KEY
    APPLE_API_KEY_PATH="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/AuthKey_${APPLE_API_KEY}.p8"
    (umask 077 && printf '%s\n' "$APPLE_API_PRIVATE_KEY" > "$APPLE_API_KEY_PATH")
    emit APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
  elif [ -n "${APPLE_ID:-}" ]; then
    need APPLE_PASSWORD APPLE_TEAM_ID
    emit APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
  else
    echo "apple-signing-env: signing without notarization; set APPLE_API_KEY or APPLE_ID to notarize" >&2
  fi
)

cmd_asset() (
  usage() {
    echo "usage: scripts/release.sh asset draft|upload|publish <tag> [files...] | verify <dir>" >&2
    exit 2
  }

  cmd="${1:-}"
  tag="${2:-}"
  [[ -n "$cmd" && -n "$tag" ]] || usage
  shift 2

  minisign_verify() {
    # shellcheck disable=SC2016
  node --input-type=module -e '
  import { createHash, createPublicKey, verify } from "node:crypto";
  import { readFileSync } from "node:fs";

  const [pubkey, file, signatureFile] = process.argv.slice(1);

  function refuse(reason) {
    console.error(reason);
    process.exit(1);
  }

  function base64(text, what) {
    const bytes = Buffer.from(text, "base64");
    if (bytes.toString("base64") !== text) refuse(`${what} is not strict base64`);
    return bytes;
  }

  const keyLine = base64(pubkey, "the public key").toString("utf8").split(/\r?\n/)[1] ?? "";
  const key = base64(keyLine, "the public key line");
  if (key.length !== 42) refuse("the public key is not a minisign key");

  const [, signatureLine = "", trusted = "", globalLine = ""] = readFileSync(signatureFile, "utf8").split(/\r?\n/);
  const signature = base64(signatureLine, "the signature line");
  const global = base64(globalLine, "the trusted comment signature");
  if (signature.length !== 74 || global.length !== 64 || !trusted.startsWith("trusted comment: ")) {
    refuse("the signature is not minisign text");
  }
  if (signature.subarray(0, 2).toString("latin1") !== "ED") refuse("the signature is not the prehashed ED algorithm");
  if (!signature.subarray(2, 10).equals(key.subarray(2, 10))) refuse("the signature is from another key");

  const publicKey = createPublicKey({
    key: { kty: "OKP", crv: "Ed25519", x: key.subarray(10).toString("base64url") },
    format: "jwk",
  });
  const body = signature.subarray(10);
  const digest = createHash("blake2b512").update(readFileSync(file)).digest();
  if (!verify(null, digest, publicKey, body)) refuse("the signature does not match the file");
  const comment = Buffer.from(trusted.slice("trusted comment: ".length), "utf8");
  if (!verify(null, Buffer.concat([body, comment]), publicKey, global)) refuse("the trusted comment does not match");
  ' -- "$@"
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

  verify_assets() {
    local assets="$1" pubkey listed digest name _ bundles signature
    pubkey=$(jq -r .plugins.updater.pubkey "$repo/tauri.conf.json")
    present "$assets" SHA256SUMS
    present "$assets" SHA256SUMS.minisig
    minisign_verify "$pubkey" "$assets/SHA256SUMS" "$assets/SHA256SUMS.minisig" ||
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
      minisign_verify "$pubkey" "$assets/$name" "$work/signature" ||
        refuse "$name does not match its signature in latest.json"
    done <<< "$bundles"
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
      verify_assets "$assets"
      if [[ "$tag" == *-* ]]; then
        flags=(--prerelease --latest=false)
      elif tag_is_latest "$tag"; then
        flags=(--latest=true)
      else
        flags=(--latest=false)
      fi
      gh release edit "$tag" --draft=false "${flags[@]}"
      ;;
    verify)
      [[ "$#" -eq 0 ]] || usage
      work=$(mktemp -d)
      trap 'rm -rf "$work"' EXIT
      verify_assets "$tag"
      ;;
    *)
      usage
      ;;
  esac
)

cmd="${1:-}"
[[ -n "$cmd" ]] || usage
shift
case "$cmd" in
  prepare) cmd_prepare "$@" ;;
  fragment) cmd_fragment "$@" ;;
  assemble-latest-json) cmd_assemble_latest_json "$@" ;;
  sign-sums) cmd_sign_sums "$@" ;;
  apple-env) cmd_apple_env "$@" ;;
  asset) cmd_asset "$@" ;;
  *) usage ;;
esac
