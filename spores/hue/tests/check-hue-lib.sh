#!/bin/sh
# Guard the one thing a copy can do quietly: drift.
#
# `lib/hue.soil` is the Spore's copy of loam's `examples/hue/hue.soil`, and the
# Spore's copy is canonical here. `lib/hue.provenance` pins the upstream commit
# and the SHA-256 of the file it was taken from. This script reports three
# states and nothing else:
#
#   ok   upstream still matches the pin, so the copy is current;
#   ok   upstream already matches the copy, so the pin is behind, not the code;
#   FAIL upstream has moved past the pin: port the change into lib/hue.soil and
#        update the pin in the same commit.
#
#   ./tests/check-hue-lib.sh                     # probes ../loam
#   SOIL=/path/to/loam ./tests/check-hue-lib.sh
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
lib="$here/lib/hue.soil"
pin_file="$here/lib/hue.provenance"

field() { sed -n "s/^$1 = \"\(.*\)\"/\1/p" "$pin_file" | head -1; }
sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
        else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

upstream_rel=$(field upstream)
pin_commit=$(field commit)
pin_sha=$(field sha256)
if [ -z "$upstream_rel" ] || [ -z "$pin_sha" ]; then
  echo "check-hue-lib.sh: lib/hue.provenance is missing upstream or sha256" >&2
  exit 1
fi

soil=${SOIL:-}
if [ -z "$soil" ]; then
  for candidate in "$here/../../../loam" "$here/../../../../singularity/soil"; do
    if [ -f "$candidate/$upstream_rel" ]; then soil=$candidate; break; fi
  done
fi
if [ -z "$soil" ] || [ ! -f "$soil/$upstream_rel" ]; then
  echo "skip: no loam checkout with $upstream_rel; set SOIL to check drift"
  exit 0
fi

upstream="$soil/$upstream_rel"
upstream_sha=$(sha "$upstream")
lib_sha=$(sha "$lib")

if [ "$upstream_sha" = "$pin_sha" ]; then
  echo "ok   upstream matches the pin at $pin_commit"
  exit 0
fi
if [ "$upstream_sha" = "$lib_sha" ]; then
  echo "ok   upstream matches the copy; bump the pin past $pin_commit"
  exit 0
fi

echo "FAIL upstream has moved past $pin_commit:" >&2
echo "     $upstream" >&2
echo "     $upstream_sha" >&2
echo "     Port the change into lib/hue.soil and update commit+sha256 in" >&2
echo "     lib/hue.provenance in the same commit. Divergence follows:" >&2
echo >&2
diff -u "$lib" "$upstream" | sed -n '1,60p' >&2 || true
exit 1
