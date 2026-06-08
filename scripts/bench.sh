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
  printf "\n=== [%d/%d] bench:%s ===\n" "$i" "${#BACKENDS[@]}" "$be"
  if [ "$#" -eq 0 ]; then
    npm run "bench:$be"
  else
    npm run "bench:$be" -- "$@"
  fi
done

printf "\n=== report ===\n"
npm run report >/dev/null 2>&1

elapsed=$(( $(date +%s) - start ))
printf "\nDone in %dm%02ds.\nReport -> %s/report/RESULTS.md\n" \
  "$(( elapsed / 60 ))" "$(( elapsed % 60 ))" "$PWD"
