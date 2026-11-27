#!/bin/sh
# Generate the deterministic contact-sheet artifacts. The marks themselves are
# produced by source/avatar.soil; this script only binds names to files and uses
# the existing `rsvg-convert` rasterizer for PNG derivatives.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SOIL_LANG=${SOIL_LANG:-$HOME/.local/share/soil/bin/soil-lang}
PRELUDE=${SOIL_PRELUDE:-$HOME/src/fungOS/soil/crates/soil-lang/stdlib/prelude.soil}
[ -x "$SOIL_LANG" ] || { echo "missing soil-lang: $SOIL_LANG" >&2; exit 1; }
[ -f "$PRELUDE" ] || { echo "missing prelude: $PRELUDE" >&2; exit 1; }
PROGRAM=$(mktemp)
trap 'rm -f "$PROGRAM"' EXIT
# 12 primary fold directions; 4 alternates expose orbit/signal families.
for n in 01 02 03 04 05 06 07 08 09 10 11 12; do
  printf '(println (avatar/render "atlas-%s" "fold"))\n' "$n" >>"$PROGRAM"
done
for spec in '13 orbit' '14 signal' '15 orbit' '16 signal'; do
  set -- $spec
  printf '(println (avatar/render "atlas-%s" "%s"))\n' "$1" "$2" >>"$PROGRAM"
done
"$SOIL_LANG" --prelude "$PRELUDE" "$HERE/source/avatar.soil" "$PROGRAM" >"$HERE/artifacts/render.tmp"
count=$(wc -l <"$HERE/artifacts/render.tmp")
[ "$count" -eq 16 ] || { echo "expected 16 marks, got $count" >&2; exit 1; }
i=0
while IFS= read -r svg; do
  i=$((i + 1))
  n=$(printf '%02d' "$i")
  printf '%s\n' "$svg" >"$HERE/artifacts/svg/atlas-$n.svg"
  rsvg-convert -w 256 -h 256 "$HERE/artifacts/svg/atlas-$n.svg" -o "$HERE/artifacts/png/atlas-$n-256.png"
  rsvg-convert -w 32 -h 32 "$HERE/artifacts/svg/atlas-$n.svg" -o "$HERE/artifacts/png/atlas-$n-32.png"
  rsvg-convert -w 24 -h 24 "$HERE/artifacts/svg/atlas-$n.svg" -o "$HERE/artifacts/png/atlas-$n-24.png"
done <"$HERE/artifacts/render.tmp"
rm -f "$HERE/artifacts/render.tmp"
echo "generated 16 SVG + 48 PNG under $HERE/artifacts"
