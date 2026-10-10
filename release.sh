#!/usr/bin/env bash
# Releases a new version: sets it, commits and pushes. GitHub then builds
# Matinee and publishes the release by itself (.github/workflows/release.yml).
#   ./release.sh 0.4.0
set -euo pipefail
cd "$(dirname "$0")"
current="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)"
version="${1:-}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Usage: ./release.sh X.Y.Z    (Matinee is $current now)"
  exit 1
fi
if [[ "$version" == "$current" ]]; then
  echo "Matinee is already $version; pick a newer version."
  exit 1
fi
sed -i "0,/^version = \".*\"/s//version = \"$version\"/" Cargo.toml
# Keep Cargo.lock in step, or GitHub's locked build refuses to start.
sed -i "/^name = \"matinee\"$/{n;s/^version = \".*\"/version = \"$version\"/}" Cargo.lock
./sync-to-github.sh "Release $version"
echo "GitHub is now building Matinee $version; it appears under Releases in a few minutes."
