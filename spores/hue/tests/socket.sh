#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# The socket half of the Hue tests: run the CLI against tests/fake_bridge.py.
#
# `tests/run.sh --socket` calls this after the pure suite, with SOIL and
# SOIL_LANG already resolved. It can also be run directly:
#
#   SOIL=/path/to/soil SOIL_LANG=/path/to/soil-lang ./tests/socket.sh
#
# The stub only accepts the key the host leases through --http-credential, so
# a green run proves the `__CREDENTIAL__` substitution happens in the host and
# not in the guest.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
: "${SOIL:?socket.sh: SOIL is required}"
: "${SOIL_LANG:?socket.sh: SOIL_LANG is required}"
soil=$SOIL
soil_lang=$SOIL_LANG

if ! command -v python3 >/dev/null 2>&1; then
  echo "socket.sh: python3 is required for the fake bridge" >&2
  exit 1
fi

work=$(mktemp -d)
server_pid=
cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

mkdir -p "$work/vfs"
export HUE_LOG="$work/requests.log"
: > "$HUE_LOG"

python3 "$here/tests/fake_bridge.py" > "$work/bridge.out" 2>&1 &
server_pid=$!

port=
for _ in $(seq 1 100); do
  if port=$(sed -n 's/^port \([0-9][0-9]*\)$/\1/p' "$work/bridge.out" 2>/dev/null) &&
     [ -n "$port" ]; then
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    echo "socket.sh: fake bridge exited before it listened" >&2
    cat "$work/bridge.out" >&2
    exit 1
  fi
  sleep 0.05
done
if [ -z "$port" ]; then
  echo "socket.sh: fake bridge did not report a port" >&2
  cat "$work/bridge.out" >&2
  exit 1
fi

export HUE_BRIDGE="http://127.0.0.1:$port"

# `hue pair` is the one request with no credential handle. A host that always
# injects a key denies it; that is a host capability gap, not a Hue failure, so
# it is detected before the suite and reported as such.
cat > "$work/pair_probe.soil" <<'PROBE'
(defun probe/pair-support ()
  (hue/cli (list "--bridge" (getenv "HUE_BRIDGE") "pair" "probe#hue")))
PROBE

files=(
  "$soil/crates/soil-lang/stdlib/prelude.soil"
  "$soil/crates/soil-lang/bindings/soil-http.soil"
  "$here/lib/hue.soil"
  "$here/lib/cli.soil"
  "$here/tests/soil/hue_socket.soil"
  "$work/pair_probe.soil"
)

run() {
  "$soil_lang" --prelude "$soil/crates/soil-lang/stdlib/prelude.soil" \
    --workspace-root "$work/vfs" --records-root "$work/records" --book hue \
    --allow "$HUE_BRIDGE" --http-credential test-key \
    "${files[@]}" --call "$1"
}

probe=$(run probe/pair-support 2>&1) || true
case "$probe" in
  "(ok"*) ;;
  *)
    echo "socket.sh: this Soil tree's host denies a credential-less request, so" >&2
    echo "socket.sh: hue pair cannot be exercised (got: $probe)" >&2
    echo "socket.sh: set SOIL to a tree whose native soil:http allows it, for" >&2
    echo "socket.sh: example the loam tip; the pure suite needs no such host." >&2
    exit 2
    ;;
esac

status=0
for name in $(grep -hoE '^\(defun (test/[^ ]+)' "$here/tests/soil/hue_socket.soil" |
              awk '{print $2}'); do
  if result=$(run "$name" 2>&1) && [ "$result" = "true" ]; then
    printf 'ok   %s\n' "$name"
  else
    printf 'FAIL %s\n' "$name"
    printf '%s\n' "$result" | sed 's/^/     /'
    status=1
  fi
done

# What crossed the wire: the scene write carries the bridge id as a string,
# and the leased key replaced the placeholder.
if ! grep -q 'PUT /api/test-key/groups/1/action {"scene": "7"}' "$HUE_LOG"; then
  echo "FAIL socket/bridge-saw-the-scene-write" >&2
  sed 's/^/     /' "$HUE_LOG" >&2
  status=1
else
  printf 'ok   socket/bridge-saw-the-scene-write\n'
fi
if grep -q '__CREDENTIAL__' "$HUE_LOG"; then
  echo "FAIL socket/placeholder-never-reached-the-bridge" >&2
  sed 's/^/     /' "$HUE_LOG" >&2
  status=1
else
  printf 'ok   socket/placeholder-never-reached-the-bridge\n'
fi

exit "$status"
