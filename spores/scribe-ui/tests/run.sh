#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Run the Scribe UI Soil tests against a disposable VFS and records book.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
root=${SCRIBE_ROOT:-$(cd "$here/../.." && pwd)}
soil=${SOIL:-$root/../soil}
soil_lang=${SOIL_LANG:-$soil/target/debug/soil-lang}
if [ ! -x "$soil_lang" ]; then
  echo "run.sh: build soil-lang first, or set SOIL_LANG" >&2
  exit 1
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/vfs"
files=(
  "$soil/crates/soil-lang/stdlib/prelude.soil"
  "$soil/crates/soil-lang/stdlib/ui.soil"
  "$soil/crates/soil-lang/bindings/soil-vfs.soil"
  "$soil/crates/soil-lang/bindings/soil-vfs-mut.soil"
  "$here/../scribe/lib/capture.soil"
  "$here/../scribe/lib/write.soil"
  "$here/ui/http.soil"
  "$here/ui/styles.soil"
  "$here/ui/views.soil"
  "$here/ui/main.soil"
  "$here/tests/soil/scribe_ui.soil"
)
run() {
  "$soil_lang" --prelude "$soil/crates/soil-lang/stdlib/prelude.soil" \
    --workspace-root "$work/vfs" --records-root "$work/records" --book scribe \
    "${files[@]}" --call "$1" >/dev/null
}
for name in $(grep -oE '^\(defun (test/[^ ]+)' "$here/tests/soil/scribe_ui.soil" | awk '{print $2}'); do
  printf 'ok %s\n' "$name"
  run "$name"
done
