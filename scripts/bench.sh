#!/usr/bin/env bash
#
# Run every backend benchmark, then regenerate report/RESULTS.md.
#
#   scripts/bench.sh                 # all benches on native, node, wasmtime
#   scripts/bench.sh udiv128         # filter forwarded to every backend
#
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

BACKENDS=(native node wasmtime)

printf "Running %d backends + report, est. ~%d min.\n" \
  "${#BACKENDS[@]}" "$(( ${#BACKENDS[@]} * 2 ))"

start=$(date +%s)
i=0

for be in "${BACKENDS[@]}"; do
  i=$(( i + 1 ))
  printf "\n-> [%d/%d] Start bench: %s <-\n\n" "$i" "${#BACKENDS[@]}" "$be"
  case "$be" in
    native)   cargo run --release -- --bench "$@" ;;
    node)     scripts/bench-node-wasi.sh "$@" ;;
    wasmtime) scripts/bench-wasmtime.sh "$@" ;;
  esac
done

printf "\n-> Prepare report <-\n\n"
python3 scripts/report.py >/dev/null 2>&1

elapsed=$(( $(date +%s) - start ))
printf "\nDone in %dm%02ds.\nReport -> %s/report/RESULTS.md\n" \
  "$(( elapsed / 60 ))" "$(( elapsed % 60 ))" "$PWD"
