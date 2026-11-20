# marbles-ui

The Marbles work graph as a server-rendered web page: a kanban by status, a
burndown against an even pace, per-project rollups, plan membership, and issue
detail. It reads a Marbles `soil:records` book and never writes it, so the
tracker stays the single writer of its own graph.

## What it shows

| Path | What |
| --- | --- |
| `/` | headline counts, a 14-day burndown, a compact board, the busiest projects |
| `/board?project=&q=&limit=` | every issue by status; unknown statuses get a column rather than disappearing |
| `/board?status=open&page=2&limit=50` | one status, paginated; the column view only draws the first 12 |
| `/burndown?days=&project=&q=` | remaining against the ideal, plus the day table |
| `/projects?stale=` | one row per project: open, WIP, review, blocked, ready, closed, stale |
| `/plans` | every `metadata.plan` value with node/ready/closed counts |
| `/plan?name=` | one plan's nodes, their readiness and what each waits on |
| `/issue?id=` | one issue: edges both ways, evidence, metadata, claim state |
| `/healthz` | `ok` |

Ready follows the same rule as `mb ready`: open, past its creation grace, not
under a live claim, and every local parent closed. "Now" is the newest moment in
the book — a Spore has no wall clock — so the same book renders the same page
whenever it is read.

## Running it

The host owns routing and the book; the Spore names neither.

```sh
soil spore run spores/marbles-ui --surface http --listen 127.0.0.1:8091 \
  --records ~/.local/share/fungos/records \
  --book personal/avery/marbles \
  --collection marbles
```

`--collection` is also the read grant, and the package declares no `[records]`
table: Marbles keeps every record type in one collection literally named
`marbles`, which is not a schema id. `[http] hosts` claims
`marbles.myc` (plus `.local` for a scratch run). To join the anycast pool:

```sh
soil spore run spores/marbles-ui --surface http --listen 0.0.0.0:8091 \
  --records ~/.local/share/fungos/records \
  --book personal/avery/marbles --collection marbles \
  --announce-endpoint http://<mesh-address>:8091 --announce-unsigned-dev
```

The publisher grant is not verified yet, which is why the unsigned-development
flag is required; once the grant path lands that flag disappears.

## Host build, warm instances and the cache

By default the host instantiates the Spore per request (`Store::new` +
`instantiate_async` in `soil`'s HTTP service), so guest memory and statics do
not survive between requests. `soil` #130 adds `--reuse-instances N`: a bounded
pool of instantiated stores, one request in flight per instance. With it, the
Spore keeps the parsed book in a `static` and every request after the first is a
cache hit.

- The cache is keyed on the newest `(revision, entity_id)` cursor in the book.
- Each request probes `records.changes(head, 1)`; an empty answer is a hit.
- On a change it re-reads, advances past tombstones, and retries if a live write
  raced the scan. A slow change feed shows up as `x-marbles-probe-ms`.
- The read itself uses `scan-packed`: one `list<u8>` per page instead of
  thousands of `entry` values. That ABI is also in #130.

Measured on the 8,807-record / 10.8 MB book, release host:

| | |
|---|---:|
| cold board request | ~50 ms packed scan + ~15 ms model |
| warm board request | **~8 ms** total |
| first request after a write | one full read, then hits |
| RSS, 1 instance | ~88 MB |
| RSS, 4 instances | ~204 MB, stable over 200 requests |

`marbles.myc` runs **one** reused instance. The records host shares one SQLite
`Store` behind a mutex, and `scan_records` parses JSON while holding it; with
several instances a cold scan can make another instance's cache probe wait tens
of milliseconds. One instance serializes requests behind an ~8 ms render instead.
`--reuse-instances N` is available when both isolation and concurrency matter
more than that tail.

Build the **host** in release. The first measurement of ~690 ms was against a
debug `soil`; the same book is 50–130 ms of packed scan in release. Debug builds
of the host, not the Spore, dominate this workload. Without `--reuse-instances`
the Spore is correct but every request is a miss (it re-reads the book); without
#130 it will not instantiate at all, because `scan-packed` is a new import.

## Boundaries

- **Read-only.** The grant is `soil:records`; there is no write path and no
  socket of the Spore's own.
- **SSR.** The guest returns complete HTML, built with `maud` (the same macro
  `spore-mail` uses), so every interpolated title or label is escaped by
  construction. No JSON fetch, no client framework, no script: what the browser
  receives is what the book said at that moment.
- **Pagination, not a dump.** The board overview draws twelve cards per column;
  a status page pages at 50 (limit up to 200). A 2,400-card column is not a page.
- **HTMX when it earns its place.** There is no HTMX today: every interaction is
  a link or a GET form. When claim/close or no-reload filtering lands, vendor
  `htmx.min.js` into the package and serve it from the Spore, as `spore-mail`
  does — never a CDN.
- **Tested on the host.** The kanban, burndown, readiness, plan grouping and
  HTML escaping live in `model` and `render`, which compile for the host, so
  `cargo test` covers them without a wasm runtime.
