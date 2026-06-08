#!/usr/bin/env bash
#
# Run every backend benchmark, then regenerate report/RESULTS.md.
#
#   scripts/bench.sh                          # all benches on native, node, wasmtime
#   scripts/bench.sh udiv128                  # filter forwarded to every backend
#   scripts/bench.sh --features native-wide-mul   # native u128 mul128, all backends
#
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

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

BACKENDS=(native node wasmtime)

printf "Running %d backends + report, est. ~%d min.\n" \
  "${#BACKENDS[@]}" "$(( ${#BACKENDS[@]} * 2 ))"

start=$(date +%s)
i=0

for be in "${BACKENDS[@]}"; do
  i=$(( i + 1 ))
  printf "\n-> [%d/%d] Start bench: %s <-\n\n" "$i" "${#BACKENDS[@]}" "$be"
  case "$be" in
    native)   cargo run --release ${feat_args[@]+"${feat_args[@]}"} -- --bench "$@" ;;
    node)     scripts/bench-node-wasi.sh ${feat_args[@]+"${feat_args[@]}"} "$@" ;;
    wasmtime) scripts/bench-wasmtime.sh ${feat_args[@]+"${feat_args[@]}"} "$@" ;;
  esac
done

printf "\n-> Prepare report <-\n\n"
python3 scripts/report.py >/dev/null 2>&1

elapsed=$(( $(date +%s) - start ))
printf "\nDone in %dm%02ds.\nReport -> %s/report/RESULTS.md\n" \
  "$(( elapsed / 60 ))" "$(( elapsed % 60 ))" "$PWD"
