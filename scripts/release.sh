#!/usr/bin/env bash
# Usage: scripts/release.sh 0.2.0
# Bumps the version, commits, tags and pushes; CI builds the release that running apps update to.
set -euo pipefail
version="${1:?usage: scripts/release.sh <version>}"
cd "$(dirname "$0")/.."

refuse() {
  echo "$1" >&2
  exit 1
}

semver='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
[[ "$version" =~ $semver ]] || refuse "the version is MAJOR.MINOR.PATCH with an optional -suffix, not $version"
[[ -z "$(git status --porcelain)" ]] || refuse "commit or stash your changes first"
branch=$(git symbolic-ref --short -q HEAD || true)
[[ "$branch" == main ]] || refuse "run a release from main, not ${branch:-a detached HEAD}"
snapshot="crates/core/wire-contract/$version.json"
[[ -z "$(git rev-parse -q --verify "refs/tags/v$version")" ]] || refuse "a local tag v$version exists. Delete it with git tag -d v$version, or pick another version."
[[ ! -e "$snapshot" ]] || refuse "$snapshot exists, so v$version already shipped. Pick another version."
git fetch -q origin main
start=$(git rev-parse HEAD)
[[ "$start" == "$(git rev-parse origin/main)" ]] || refuse "HEAD is not origin/main. Pull or push main, then run the release again."
remote_tag=$(git ls-remote origin "refs/tags/v$version")
[[ -z "$remote_tag" ]] || refuse "origin already has v$version. Pick another version."

wrote_toml=
wrote_lock=
edited() {
  [[ -n "$2" && "$(git hash-object -- "$1")" != "$2" ]]
}
put_back() {
  if edited "$1" "$2"; then
    echo "$1 changed while the release ran, so it keeps your edit and the v$version bump." >&2
  else
    git checkout -q HEAD -- "$1"
  fi
}
restore() {
  put_back Cargo.toml "$wrote_toml"
  put_back Cargo.lock "$wrote_lock"
  git rm -q --cached --ignore-unmatch -- "$snapshot"
  rm -f "$snapshot"
}
trap restore EXIT
sed -i.bak "s/^version = \".*\"/version = \"$version\"/" Cargo.toml
rm Cargo.toml.bak
wrote_toml=$(git hash-object Cargo.toml)
cargo update -p pinned --quiet
wrote_lock=$(git hash-object Cargo.lock)
if [[ "$version" == *-* ]]; then
  echo "v$version is a release candidate. Hub deploy skips it, so it gets no wire snapshot."
else
  cargo run -q -p pinned-server --bin wire-contract -- snapshot "$version"
  git add "$snapshot"
fi
stray=$(
  git status --porcelain -- ':!Cargo.toml' ':!Cargo.lock' ":!$snapshot"
  if edited Cargo.toml "$wrote_toml"; then echo " M Cargo.toml"; fi
  if edited Cargo.lock "$wrote_lock"; then echo " M Cargo.lock"; fi
)
[[ -z "$stray" ]] || refuse "the release stopped before the commit, because these files changed while the release built:
$stray"
git add Cargo.toml Cargo.lock
git commit -m "Release v$version"
trap - EXIT
git tag "v$version"
if ! git push --atomic origin HEAD:refs/heads/main "refs/tags/v$version"; then
  remote_tag=$(git ls-remote origin "refs/tags/v$version" | cut -f1) ||
    refuse "git push failed and origin cannot be read, so main keeps the release commit and v$version. Check origin before you run the release again."
  [[ "$remote_tag" != "$(git rev-parse "v$version")" ]] ||
    refuse "git push failed, but origin has v$version, so the release went out. The tag on origin is at the release commit. Fetch origin before you change anything."
  git reset -q --keep "$start" ||
    refuse "the push was rejected, so the release did not go out. Local main keeps the release commit and v$version, because these files changed while the push ran:
$(git status --porcelain)
Stash them, then run: git tag -d v$version && git reset --keep $start && git stash pop"
  git tag -d "v$version" > /dev/null
  [[ -z "$remote_tag" ]] ||
    refuse "the push was rejected, and origin has v$version at another commit. The release did not go out, and main is back where it started. Check that tag on origin before you pick a version."
  refuse "the push was rejected, so origin has no new main and no v$version. Pull main, then run the release again."
fi
echo "Pushed v$version. Watch the build with: gh run watch"
