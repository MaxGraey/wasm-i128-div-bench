#!/usr/bin/env bash
#
# Build the criterion benchmark to wasm32-wasip1 and run it under node:wasi.
#
#   scripts/bench-node-wasi.sh                          # all benchmarks
#   scripts/bench-node-wasi.sh udiv128                  # filter by name
#   scripts/bench-node-wasi.sh --sample-size 50 reciprocal
#
# Any extra arguments are forwarded to criterion. --bench is injected so
# criterion measures instead of dropping into its verify-only test mode.
#
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --target wasm32-wasip1

WASM="$PWD/target/wasm32-wasip1/release/wasm-i128-div-bench.wasm"
DATA="$PWD/target/wasi"
mkdir -p "$DATA"

WASI_BENCH_DIR="$DATA" node \
  --experimental-wasi-unstable-preview1 \
  --disable-warning=ExperimentalWarning \
  scripts/run-node-wasi.mjs "$WASM" --bench "$@"
