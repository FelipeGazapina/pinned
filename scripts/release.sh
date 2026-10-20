#!/usr/bin/env bash
# Usage: scripts/release.sh 0.2.0
# Bumps the version, commits, tags and pushes; CI builds the release that running apps update to.
set -euo pipefail
version="${1:?usage: scripts/release.sh <version>}"
cd "$(dirname "$0")/.."

[[ -z "$(git status --porcelain)" ]] || { echo "commit or stash your changes first" >&2; exit 1; }
sed -i '' "s/^version = \".*\"/version = \"$version\"/" Cargo.toml
cargo update -p pinned --quiet
git commit -am "Release v$version"
git tag "v$version"
git push origin HEAD "v$version"
echo "Pushed v$version. Watch the build with: gh run watch"
