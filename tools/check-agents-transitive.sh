#!/bin/sh
# Regression: a composed consumer must include its provider's transitive
# providers, not only the direct provider. This is the `agents-cli` -> `agents`
# -> theseus/acp case.
#
# Scratch root only; no live install, no restart. Exits 1 with a precise missing
# list until Loam's composer follows the new imports introduced by a provider.
set -eu

ROOT=${FUNGOS_SPORE_ROOT:-"$HOME/.local/share/fungos/spores"}
SPORES=${SPORES_SOURCE:-"$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)/spores"}
LOAM=${LOAM:-loam}
TMP=${TMPDIR:-/tmp}/agents-transitive-check.$$

cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM

command -v jq >/dev/null || { echo "jq is required" >&2; exit 2; }
command -v "$LOAM" >/dev/null || { echo "loam is required" >&2; exit 2; }
[ -d "$ROOT" ] || { echo "no live Spore root at $ROOT" >&2; exit 2; }

mkdir -p "$TMP/.loam"
[ -d "$ROOT/.loam/compositions" ] && cp -R "$ROOT/.loam/compositions" "$TMP/.loam/compositions"

"$LOAM" install "$SPORES/agents" --root "$TMP" --provider-root "$ROOT" --allow-unsigned-dev >/dev/null
"$LOAM" install "$SPORES/agents-cli" --root "$TMP" --provider-root "$TMP" --provider-root "$ROOT" --allow-unsigned-dev >/dev/null

providers=$(jq -r '.providers[] | select(.id != null) | .id' "$TMP/agents-cli/runtime.json" | sort -u)
echo "agents-cli runtime providers:"
printf '%s\n' "$providers" | sed 's/^/  /'

missing=""
for required in dev.mycelium.theseus-agents dev.mycelium.acp-agents; do
  printf '%s\n' "$providers" | grep -qx "$required" || missing="$missing $required"
done
if [ -n "$missing" ]; then
  echo "TRANSITIVE-MISSING:$missing" >&2
  echo "the composer linked only the direct agent provider; its new imports were not followed" >&2
  exit 1
fi
echo "PASS: agents-cli runtime carries the transitive agent adapters"
