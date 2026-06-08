#!/usr/bin/env bash
#
# Build the criterion benchmark to wasm32-wasip1 and run it under wasmtime with
# the wide-arithmetic proposal enabled.
#
#   scripts/bench-wasmtime.sh                          # all benchmarks
#   scripts/bench-wasmtime.sh udiv128                  # filter by name
#   scripts/bench-wasmtime.sh --sample-size 50 reciprocal
#
# Any extra arguments are forwarded to criterion. --bench is injected so
# criterion measures instead of dropping into its verify-only test mode.
#
# build-std (.cargo/config.toml) rebuilds std / compiler-builtins with
# +wide-arithmetic too, so the builtin __udivti3 / __umodti3 baseline is measured
# with the proposal as well.
#
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --target wasm32-wasip1

WASM="$PWD/target/wasm32-wasip1/release/wasm-i128-div-bench.wasm"
DATA="$PWD/target/wasmtime"
mkdir -p "$DATA"

# wasmtime from PATH, falling back to the default installer location.
WASMTIME="${WASMTIME:-$(command -v wasmtime || echo "$HOME/.wasmtime/bin/wasmtime")}"

# Force the optimizing backend (cranelift, not the winch baseline) at max
# opt-level, and enable wide-arithmetic so i64.add128 / i64.sub128 /
# i64.mul_wide_u are accepted.
"$WASMTIME" run \
  -O opt-level=2 \
  -C compiler=cranelift \
  -W wide-arithmetic=y \
  --dir "$DATA" \
  --env CARGO_TARGET_DIR="$DATA" \
  "$WASM" --bench "$@"
