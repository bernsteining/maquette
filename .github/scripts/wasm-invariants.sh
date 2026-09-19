#!/usr/bin/env bash
# Guards two shipped-artifact invariants for a plugin wasm:
#   1. it imports ONLY the two Typst protocol functions (no WASI, no other host
#      calls sneaking in via a dependency) — the "zero host imports" promise;
#   2. it stays under a per-plugin size budget (headroom over today's size).
# Uses wasm-dis (binaryen), already on PATH in the CI image.
set -euo pipefail

wasm="${1:?usage: wasm-invariants.sh <path-to-wasm>}"

case "$(basename "$wasm")" in
  maquette.wasm)       max=700000 ;;
  maquette-gltf.wasm)  max=2000000 ;;
  maquette-scad.wasm)  max=1300000 ;;
  *) echo "::error::no size budget defined for $(basename "$wasm")"; exit 1 ;;
esac

extra=$(wasm-dis "$wasm" \
  | grep -oE '\(import "[^"]+" "[^"]+"' \
  | grep -vE 'typst_env" "wasm_minimal_protocol_(send_result_to_host|write_args_to_buffer)' \
  | sort -u || true)
if [ -n "$extra" ]; then
  echo "::error::$wasm has unexpected host imports (only the Typst protocol is allowed):"
  echo "$extra"
  exit 1
fi

size=$(stat -c%s "$wasm")
if [ "$size" -gt "$max" ]; then
  echo "::error::$wasm is $size bytes, over budget $max — investigate the bloat or raise the budget"
  exit 1
fi

echo "$wasm ✓ imports=protocol-only, size=$size bytes (budget $max)"
