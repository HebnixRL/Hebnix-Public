#!/usr/bin/env bash

# Stamp the release version (from the git tag) into every place that carries it,
# so nobody bumps versions by hand. Run from anywhere: stamp-version.sh 2.2.7

set -euo pipefail

V="${1:?usage: stamp-version.sh <x.y.z>}"
[[ "$V" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not x.y.z: $V" >&2; exit 1; }

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# first `version = ` line of each manifest is the package/workspace version
for f in Cargo.toml crates/hebnix-sdk/Cargo.toml crates/hebnix-app/Cargo.toml; do
    sed -i "0,/^version = \"[^\"]*\"/s//version = \"$V\"/" "$f"
done
sed -i "s/\(hebnix-sdk = { version = \"\)[^\"]*\"/\1$V\"/" crates/hebnix-app/Cargo.toml
sed -i "s/\(pub const APP_VERSION: &str = \"\)[^\"]*\"/\1$V\"/" \
    crates/hebnix-app/src/app.rs crates/hebnix-app/src/lite_app.rs

grep -rn "$V" Cargo.toml crates/*/Cargo.toml crates/hebnix-app/src/app.rs crates/hebnix-app/src/lite_app.rs | grep -c . | xargs echo "stamped $V in places:"
