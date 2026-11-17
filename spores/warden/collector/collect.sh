#!/bin/sh
# collect.sh: the fleet in two commands, for a machine with `gh` and no daemon.
#
#   ./collector/collect.sh FuturePresentLabs mycelium-linux > snapshot.json
#   warden report < snapshot.json
#
# This is the reference collector, not the only one. It exists to prove the
# snapshot contract is writable by anything: a shell here, a Spore over
# `soil:http` later, a scheduled job on a box that already holds a token. The
# mapping from forge payload to snapshot is in `warden ingest`, not in this
# script, which is why the script is nine lines and the mapping has tests.
#
# The token is the caller's: `gh` already holds one, and warden never sees it.
set -eu

if [ "$#" -eq 0 ]; then
  echo "usage: collect.sh ORG... [--limit N]" >&2
  exit 2
fi

here=$(dirname "$0")
query=$(cat "$here/fleet.graphql")

for org in "$@"; do
  echo "collecting $org" >&2
  gh api graphql --paginate -f org="$org" -f "query=$query"
done
