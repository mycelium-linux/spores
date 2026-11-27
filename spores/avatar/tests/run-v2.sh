#!/bin/sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SOIL_LANG=${SOIL_LANG:-$HOME/.local/share/soil/bin/soil-lang}
PRELUDE=${SOIL_PRELUDE:-$HOME/src/fungOS/soil/crates/soil-lang/stdlib/prelude.soil}
OUT=$("$SOIL_LANG" --prelude "$PRELUDE" "$HERE/source/avatar.soil" "$HERE/source/avatar-v2.soil" "$HERE/tests/avatar-v2.soil")
printf '%s\n' "$OUT"
printf '%s\n' "$OUT" | grep -q 'FAIL' && exit 1
exit 0
