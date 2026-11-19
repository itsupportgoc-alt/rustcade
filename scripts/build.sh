#!/usr/bin/env bash
# Build the Rust core to WebAssembly and publish it next to the web app.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build -p rustcade-wasm --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/rustcade_wasm.wasm web/emulator.wasm

size=$(wc -c < web/emulator.wasm)
echo "built web/emulator.wasm (${size} bytes)"
echo "run ./scripts/serve.sh and open the printed URL on your phone"
