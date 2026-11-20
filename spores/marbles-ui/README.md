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
