#!/bin/sh
# Enforce the UI contract decision.
#
# - `soil:surface` is the canonical spore-facing presentation contract. A Spore
#   that declares it must declare it in both `[engine].abi` and `[grants]`, and
#   must not also declare `soil:canvas` or `soil:present`.
# - Lua `[ui]` is frozen. The baseline names the packages that still have one;
#   a new one fails until the port list changes.
# - `soil:canvas` and `soil:present` are legacy. The baseline names their
#   current consumers; a new consumer fails.
#
# The baseline is deliberately exact: removing a Spore or migrating it means
# deleting its row in the same change. A stale row is a lie about the tree.
set -eu

cd "$(dirname "$0")/.."
baseline=tools/ui-contract-baseline.txt
[ -f "$baseline" ] || {
  echo "missing $baseline" >&2
  exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

find spores apps -name spore.toml -type f | sort >"$work/manifests"

has_abi() {
  # has_abi <manifest> <name> <version>
  grep -Eq "(^|[ ,{])$2[[:space:]]*=[[:space:]]*\"$3\"" "$1"
}

has_grant() {
  # has_grant <manifest> <package>
  grep -Eq "^\"$2\"[[:space:]]*=" "$1"
}

: >"$work/current"
status=0
while read -r manifest; do
  dir=$(dirname "$manifest")
  : >"$work/wit"
  if [ -d "$dir/wit" ]; then
    find "$dir/wit" -maxdepth 1 -name '*.wit' -type f -exec cat {} + >"$work/wit"
  fi

  if grep -qx '\[ui\]' "$manifest"; then
    echo "lua-ui $dir" >>"$work/current"
  fi

  surface=0
  if has_abi "$manifest" surface 1.1.0; then surface=1; fi
  if has_grant "$manifest" soil:surface; then surface=1; fi
  if grep -qE '^[[:space:]]*import[[:space:]]+soil:surface/' "$work/wit"; then surface=1; fi
  if [ "$surface" -eq 1 ]; then
    if ! has_abi "$manifest" surface 1.1.0 || ! has_grant "$manifest" soil:surface; then
      echo "surface: $dir must declare both abi.surface = \"1.1.0\" and grant \"soil:surface\"" >&2
      status=1
    fi
    if has_abi "$manifest" canvas 1.0.0 || has_grant "$manifest" soil:canvas || grep -qE '^[[:space:]]*import[[:space:]]+soil:canvas/' "$work/wit"; then
      echo "surface: $dir also declares legacy soil:canvas" >&2
      status=1
    fi
    if has_abi "$manifest" present 0.1.0 || has_grant "$manifest" soil:present || grep -qE '^[[:space:]]*import[[:space:]]+soil:present/' "$work/wit"; then
      echo "surface: $dir also declares legacy soil:present" >&2
      status=1
    fi
    echo "surface $dir" >>"$work/current"
  fi

  if has_abi "$manifest" canvas 1.0.0 || has_grant "$manifest" soil:canvas || grep -qE '^[[:space:]]*import[[:space:]]+soil:canvas/' "$work/wit"; then
    echo "legacy-canvas $dir" >>"$work/current"
  fi
  if has_abi "$manifest" present 0.1.0 || has_grant "$manifest" soil:present || grep -qE '^[[:space:]]*import[[:space:]]+soil:present/' "$work/wit"; then
    echo "legacy-present $dir" >>"$work/current"
  fi
done <"$work/manifests"
sort -u "$work/current" -o "$work/current"

awk '!/^[[:space:]]*#/ && NF == 2 { print $1, $2 }' "$baseline" | sort -u >"$work/baseline"

status=0

while read -r kind path; do
  case "$kind" in
    surface) continue ;;
    lua-ui|legacy-canvas|legacy-present)
      if ! grep -qxF "$kind $path" "$work/baseline"; then
        echo "new $kind: $path is not in $baseline" >&2
        status=1
      fi
      ;;
    *)
      echo "unknown baseline kind: $kind" >&2
      status=1
      ;;
  esac
done <"$work/current"

while read -r kind path; do
  case "$kind" in
    lua-ui|legacy-canvas|legacy-present)
      if ! grep -qxF "$kind $path" "$work/current"; then
        echo "stale $kind: $path is in $baseline but no longer declares it" >&2
        status=1
      fi
      ;;
  esac
done <"$work/baseline"

exit $status
