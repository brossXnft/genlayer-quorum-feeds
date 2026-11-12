#!/usr/bin/env bash
# Build the contract exactly as deployed: wasm32-wasip1 release, wasm-opt -Oz,
# genvm.version section. Prints the SHA-256 to compare with the deployed code.
set -euo pipefail
cd "$(dirname "$0")/.."
WASM_OPT=${WASM_OPT:-wasm-opt}
cargo build --release --target wasm32-wasip1 -p quorum-feeds
mkdir -p dist
"$WASM_OPT" -Oz --enable-bulk-memory --enable-sign-ext --enable-mutable-globals \
  --enable-nontrapping-float-to-int target/wasm32-wasip1/release/quorum-feeds.wasm -o dist/quorum-feeds.opt.wasm
python3 tools/wasm_strip.py dist/quorum-feeds.opt.wasm dist/quorum-feeds.wasm --version v0.3.0 >/dev/null
rm dist/quorum-feeds.opt.wasm
sha256sum dist/quorum-feeds.wasm
