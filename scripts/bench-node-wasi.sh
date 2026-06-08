#!/usr/bin/env bash
#
# Build the criterion benchmark to wasm32-wasip1 and run it under node:wasi.
#
#   scripts/bench-node-wasi.sh                          # all benchmarks
#   scripts/bench-node-wasi.sh udiv128                  # filter by name
#   scripts/bench-node-wasi.sh --sample-size 50 reciprocal
#   scripts/bench-node-wasi.sh --features native-wide-mul   # native u128 mul128
#
# Any extra arguments are forwarded to criterion. --bench is injected so
# criterion measures instead of dropping into its verify-only test mode.
#
set -euo pipefail
cd "$(dirname "$0")/.."

# Optional `--features <name>` goes to cargo; the rest stay as criterion filters.
feat_args=()
rest=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --features)   feat_args=(--features "$2"); shift 2 ;;
    --features=*) feat_args=(--features "${1#*=}"); shift ;;
    *)            rest+=("$1"); shift ;;
  esac
done
set -- ${rest[@]+"${rest[@]}"}

cargo build --release ${feat_args[@]+"${feat_args[@]}"} --target wasm32-wasip1

WASM="$PWD/target/wasm32-wasip1/release/wasm-i128-div-bench.wasm"
DATA="$PWD/target/wasi"
mkdir -p "$DATA"

# Force TurboFan-only wasm compilation (--no-liftoff drops the Liftoff baseline)
# and compile eagerly (--no-wasm-lazy-compilation), so the benchmark runs fully
# optimized from the first iteration instead of tiering up mid-run.
WASI_BENCH_DIR="$DATA" node \
  --no-liftoff \
  --no-wasm-lazy-compilation \
  --experimental-wasi-unstable-preview1 \
  --disable-warning=ExperimentalWarning \
  scripts/run-node-wasi.mjs "$WASM" --bench "$@"
