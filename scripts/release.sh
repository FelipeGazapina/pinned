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
git fetch -q origin main
start=$(git rev-parse HEAD)
[[ "$start" == "$(git rev-parse origin/main)" ]] || refuse "HEAD is not origin/main. Pull or push main, then run the release again."

sed -i.bak "s/^version = \".*\"/version = \"$version\"/" Cargo.toml
rm Cargo.toml.bak
cargo update -p pinned --quiet
# Hub deploy skips a suffixed tag, so a release candidate gets no snapshot that every later hub must honor.
if [[ "$version" != *-* ]]; then
  # Copies current.json and fails when that file does not match the code.
  cargo run -q -p pinned-server --bin wire-contract -- snapshot "$version"
  # `git commit -am` leaves a new file out.
  git add "crates/core/wire-contract/${version}.json"
fi
git commit -am "Release v$version"
git tag "v$version"
if ! git push --atomic origin HEAD:refs/heads/main "refs/tags/v$version"; then
  git tag -d "v$version" > /dev/null
  git reset -q --hard "$start"
  refuse "the push was rejected, so origin has no new main and no v$version. Pull main, then run the release again."
fi
echo "Pushed v$version. Watch the build with: gh run watch"
