#!/bin/bash
# Build release binaries for M4 distribution targets.
# Usage: scripts/release.sh [version]
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="${1:-$(cargo metadata --no-deps --format-version 1 | python3 -c 'import sys,json;print(json.load(sys.stdin)["packages"][0]["version"])')}"
OUT="dist/v${VERSION}"
mkdir -p "$OUT"
for target in aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  echo "== $target =="
  rustup target add "$target" 2>/dev/null || true
  cargo build --release --target "$target" 2>/dev/null || { echo "skip $target (toolchain unavailable)"; continue; }
  tar -czf "$OUT/reran-v${VERSION}-${target}.tar.gz" -C "target/${target}/release" reran
  shasum -a 256 "$OUT/reran-v${VERSION}-${target}.tar.gz" >> "$OUT/SHA256SUMS"
done
echo "artifacts in $OUT"
