#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ ! -f web/emulator.wasm ]; then
  echo "web/emulator.wasm is missing, building it first..."
  ./scripts/build.sh
fi
exec node scripts/serve.mjs "$@"
