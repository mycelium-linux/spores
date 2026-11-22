#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Run the Hue Spore's Soil tests: colour theory, spatial planning, request
# shaping, and bridge error decoding. No bridge, no socket.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
root=${SPORES_ROOT:-$(cd "$here/../.." && pwd)}

# The Soil tree that owns crates/soil-lang and the generated soil:http
# binding. Override with SOIL; otherwise probe the checkouts beside this repo.
find_soil() {
  if [ -n "${SOIL:-}" ]; then
    printf '%s\n' "$SOIL"
    return 0
  fi
  local candidate
  for candidate in "$root/../loam" "$root/../soil" "$root/../soil-dsp" \
                   "$root/../../singularity/soil"; do
    if [ -f "$candidate/crates/soil-lang/bindings/soil-http.soil" ]; then
      (cd "$candidate" && pwd)
      return 0
    fi
  done
  return 1
}

if ! soil=$(find_soil); then
  echo "run.sh: no Soil tree with crates/soil-lang/bindings/soil-http.soil;" >&2
  echo "run.sh: set SOIL=/path/to/soil" >&2
  exit 1
fi
soil_lang=${SOIL_LANG:-$soil/target/debug/soil-lang}
if [ ! -x "$soil_lang" ]; then
  echo "run.sh: build soil-lang first ($soil_lang), or set SOIL_LANG" >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/vfs"

files=(
  "$soil/crates/soil-lang/stdlib/prelude.soil"
  "$soil/crates/soil-lang/bindings/soil-http.soil"
  "$here/lib/hue.soil"
  "$here/tests/soil/hue.soil"
  "$here/tests/soil/hue_request.soil"
)

run() {
  "$soil_lang" --prelude "$soil/crates/soil-lang/stdlib/prelude.soil" \
    --workspace-root "$work/vfs" --records-root "$work/records" --book hue \
    "${files[@]}" --call "$1"
}

status=0
for name in $(grep -hoE '^\(defun (test/[^ ]+)' "${files[@]}" | awk '{print $2}'); do
  if result=$(run "$name" 2>&1) && [ "$result" = "true" ]; then
    printf 'ok   %s\n' "$name"
  else
    printf 'FAIL %s\n' "$name"
    printf '%s\n' "$result" | sed 's/^/     /'
    status=1
  fi
done
exit "$status"
