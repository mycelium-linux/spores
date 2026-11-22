#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Run the Scribe spore's Soil tests against a disposable VFS and records book.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
root=${SCRIBE_ROOT:-$(cd "$here/../.." && pwd)}
soil=${SOIL:-$root/../soil}

first() {
  for path in "$@"; do
    if [ -e "$path" ]; then printf '%s\n' "$path"; return 0; fi
  done
  return 1
}

soil_lang=${SOIL_LANG:-}
if [ -z "$soil_lang" ]; then
  soil_lang=$(first "$soil/target/debug/soil-lang" "$soil/target/release/soil-lang") || {
    echo "run.sh: build soil-lang first, or set SOIL_LANG" >&2
    exit 1
  }
fi
prelude=$(first "$soil/crates/soil-lang/stdlib/prelude.soil" "$soil/core/prelude.soil")
ui=$(first "$soil/crates/soil-lang/stdlib/ui.soil" "$soil/core/ui.soil")
bindings=$soil/crates/soil-lang/bindings

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/vfs"
files=(
  "$prelude"
  "$ui"
  "$bindings/soil-vfs.soil"
  "$bindings/soil-vfs-mut.soil"
  "$here/source/lib/capture.soil"
  "$here/source/lib/write.soil"
  "$here/source/ui/http.soil"
  "$here/source/ui/styles.soil"
  "$here/source/ui/views.soil"
  "$here/source/ui/main.soil"
  "$here/tests/soil/scribe_ui.soil"
)
run() {
  "$soil_lang" --prelude "$prelude" \
    --workspace-root "$work/vfs" --records-root "$work/records" --book scribe \
    "${files[@]}" --call "$1" >/dev/null
}
for name in $(grep -oE '^\(defun (test/[^ ]+)' "$here/tests/soil/scribe_ui.soil" | awk '{print $2}'); do
  printf 'ok %s\n' "$name"
  run "$name"
done
