#!/bin/sh
# Pure source checks for the Hue SSR Spore: no bridge, no socket.
set -eu
cd "$(dirname "$0")"

SOIL=${SOIL:-$HOME/src/singularity/soil}
SOIL_LANG=${SOIL_LANG:-$HOME/.local/share/soil/bin/soil-lang}
SOIL_HTTP=${SOIL_HTTP:-$SOIL/std/bindings/soil-http.soil}
SOIL_VFS=${SOIL_VFS:-$SOIL/std/bindings/soil-vfs.soil}

[ -x "$SOIL_LANG" ] || { echo "missing $SOIL_LANG" >&2; exit 2; }

"$SOIL_LANG" \
  --prelude "$SOIL/core/prelude.soil" \
  "$SOIL/core/ui.soil" \
  "$SOIL/core/url.soil" \
  "$SOIL_HTTP" "$SOIL_VFS" \
  ../../lib/hue.soil \
  ../../lib/cli.soil \
  ../source/render.soil \
  ../source/routes.soil \
  ../source/config.soil \
  render.soil \
  --call test/all
