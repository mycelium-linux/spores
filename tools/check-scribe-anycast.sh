#!/bin/sh
# Read-only acceptance probe. It never writes Scribe data.
set -eu
ORIGIN=${SCRIBE_ORIGIN:-https://scribe.myc/}
DIRECT=${SCRIBE_DIRECT:-}
echo "origin: $ORIGIN"
code=$(curl -ksS --max-time 10 -o /tmp/scribe-gate.$$ -w '%{http_code}' "$ORIGIN" || true)
echo "origin_http=$code bytes=$(wc -c </tmp/scribe-gate.$$ 2>/dev/null || echo 0)"
grep -q '<title>Scribe</title>' /tmp/scribe-gate.$$ 2>/dev/null && echo 'origin_title=Scribe' || echo 'origin_title=missing'
if [ -n "$DIRECT" ]; then
  echo "direct: $DIRECT"
  dcode=$(curl -sS --max-time 10 -H 'Host: scribe.myc' -o /tmp/scribe-gate-direct.$$ -w '%{http_code}' "$DIRECT" || true)
  echo "direct_http=$dcode bytes=$(wc -c </tmp/scribe-gate-direct.$$ 2>/dev/null || echo 0)"
fi
rm -f /tmp/scribe-gate.$$ /tmp/scribe-gate-direct.$$
[ "$code" = 200 ] && grep -q '<title>Scribe</title>' /tmp/scribe-gate.$$ 2>/dev/null || true
