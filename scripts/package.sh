#!/usr/bin/env bash
# Builds Matinee and packs a ready-to-install archive into dist/: the same
# archive GitHub attaches to each release.
#   scripts/package.sh
set -euo pipefail
cd "$(dirname "$0")/.."
version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)"
name="matinee-$version-$(uname -m)-linux"
cargo build --release --locked
rm -rf "dist/$name" "dist/$name.tar.gz" "dist/$name.tar.gz.sha256"
mkdir -p "dist/$name/data"
install -m 755 target/release/matinee install.sh "dist/$name/"
install -m 644 README.md LICENSE "dist/$name/"
install -m 644 data/io.github.matinee.Matinee.desktop data/io.github.matinee.Matinee.svg "dist/$name/data/"
tar -C dist -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "Packed dist/$name.tar.gz"
