#!/usr/bin/env bash
#
# Build the criterion benchmark to wasm32-wasip1 and run it under d8 (V8) with
# the wide-arithmetic proposal enabled.
#
#   scripts/bench-d8.sh                          # all benchmarks
#   scripts/bench-d8.sh udiv128                  # filter by name
#   scripts/bench-d8.sh --sample-size 50 reciprocal
#
# node:wasi cannot run this: stock Node rejects the wide-arithmetic opcodes. d8
# accepts them under --experimental-wasm-wide-arithmetic, but has no WASI, so a
# minimal shim (scripts/run-d8-wasi.mjs + @bjorn3/browser_wasi_shim) supplies the
# monotonic clock, argv, stdout and a writable in-memory preopen. criterion's
# result files live only in that in-memory FS, so the driver prints them on a
# marker line and the step below writes them under target/d8 for report.py.
#
# build-std (.cargo/config.toml) rebuilds std / compiler-builtins with
# +wide-arithmetic too, so the builtin __udivti3 / __umodti3 baseline is measured
# with the proposal as well.
#
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --target wasm32-wasip1

WASM="$PWD/target/wasm32-wasip1/release/wasm-i128-div-bench.wasm"
DATA="$PWD/target/d8"
mkdir -p "$DATA"

# d8 from jsvu (~/.jsvu/bin/v8), overridable via $V8.
V8="${V8:-$(command -v v8 || echo "$HOME/.jsvu/bin/v8")}"

OUT="$DATA/output.txt"

# --no-liftoff forces TurboFan (no baseline tier), matching a fully-compiled JIT.
"$V8" \
  --no-liftoff \
  --experimental-wasm-wide-arithmetic \
  --module scripts/run-d8-wasi.mjs \
  -- "$WASM" --bench "$@" > "$OUT" 2>&1

# Show criterion's human-readable output, minus the data marker and shim chatter.
grep -av -e '===CRITERION-DATA===' -e '%c%s color' "$OUT" || true

# Persist the estimates the driver dumped, so report.py reads d8 like the others.
python3 - "$DATA" "$OUT" <<'PY'
import sys, json, os

data_dir, out = sys.argv[1], sys.argv[2]
marker = "===CRITERION-DATA==="
blob = None
for line in open(out, encoding="utf-8", errors="replace"):
    k = line.find(marker)
    if k >= 0:
        blob = line[k + len(marker):]
        break

if not blob:
    sys.exit("d8 run produced no criterion data (did it crash?)")

files = json.loads(blob)
for rel, content in files.items():
    dest = os.path.join(data_dir, rel)
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    with open(dest, "w", encoding="utf-8") as f:
        f.write(content)

print(f"persisted {len(files)} estimate files under {data_dir}", file=sys.stderr)
PY
