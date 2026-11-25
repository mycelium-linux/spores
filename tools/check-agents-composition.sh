#!/bin/sh
# Regression check: a provider change without a version bump must recompose the
# `agents` aggregator on reinstall, without bypassing the composition cache.
#
# It never writes the live install root. It copies the live composition cache
# into a scratch root, installs the checked-in `agents` package there against
# the live providers, and checks that every provider linked in the resulting
# runtime matches the provider bytes currently installed.
#
# STRICT=1 also fails when the live aggregator is already stale, which is the
# host-audit mode. The default mode proves that a reinstall would fix it.
set -eu

ROOT=${FUNGOS_SPORE_ROOT:-"$HOME/.local/share/fungos/spores"}
PKG=${AGENTS_PACKAGE:-"$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)/spores/agents"}
LOAM=${LOAM:-loam}
TMP=${TMPDIR:-/tmp}/agents-composition-check.$$

cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM

command -v jq >/dev/null || { echo "jq is required" >&2; exit 2; }
command -v "$LOAM" >/dev/null || { echo "loam is required" >&2; exit 2; }

agents_link=$(readlink "$ROOT/agents" 2>/dev/null || true)
[ -n "$agents_link" ] || { echo "no installed agents Spore in $ROOT" >&2; exit 2; }
agents_dir="$ROOT/$agents_link"
[ -f "$agents_dir/runtime.json" ] || { echo "no runtime.json in $agents_dir" >&2; exit 2; }

# Print `id expected actual` for every provider whose linked module bytes differ
# from what the aggregator recorded. Transitive providers are included because
# they all appear in runtime.json.
mismatches() {
  runtime=$1
  jq -r '.providers[] | select(.module_path != null) | [.id, .module, .module_path] | @tsv' "$runtime" |
  while IFS="$(printf '\t')" read -r id expected path; do
    [ -f "$path" ] || { echo "$id missing $path"; continue; }
    actual=$(shasum -a 256 "$path" | awk '{print $1}')
    [ "$actual" = "$expected" ] || echo "$id $expected $actual"
  done | sort -u
}

live_runtime=$(shasum -a 256 "$agents_dir/runtime.wasm" | awk '{print $1}')
live_mismatches=$(mismatches "$agents_dir/runtime.json" || true)
echo "live agents runtime: $live_runtime"
if [ -z "$live_mismatches" ]; then
  echo "RESULT: live aggregator links every current provider byte"
  exit 0
fi
echo "RESULT: live aggregator is stale:"
printf '%s\n' "$live_mismatches" | sed 's/^/  /'

mkdir -p "$TMP/.loam"
if [ -d "$ROOT/.loam/compositions" ]; then
  cp -R "$ROOT/.loam/compositions" "$TMP/.loam/compositions"
fi

"$LOAM" install "$PKG" --root "$TMP" --provider-root "$ROOT" --allow-unsigned-dev >/dev/null

new_runtime=$(shasum -a 256 "$TMP/agents/runtime.wasm" | awk '{print $1}')
new_mismatches=$(mismatches "$TMP/agents/runtime.json" || true)
echo "reinstalled runtime: $new_runtime"
if [ -n "$new_mismatches" ]; then
  echo "FAIL: reinstall still links stale provider bytes:" >&2
  printf '%s\n' "$new_mismatches" | sed 's/^/  /' >&2
  exit 1
fi
if [ "$new_runtime" = "$live_runtime" ]; then
  echo "FAIL: reinstall reused the stale runtime" >&2
  exit 1
fi
echo "PASS: cache-preserving reinstall recomposed every provider link"
if [ "${STRICT:-0}" = "1" ]; then
  echo "STRICT: live aggregator is stale; reinstall the consumer" >&2
  exit 1
fi
