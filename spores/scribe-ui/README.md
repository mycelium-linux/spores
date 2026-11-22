# Scribe UI

A browser view over Scribe capture bundles, written in Soil.

This spore is the UI half of the [Scribe Soil port](../../docs/scribe-soil-port.md).
It renders server-side from `el`/`attr`/`txt` forms and has no client
JavaScript and no Rust request handler.

```text
ui/http.soil     request parsing and response writing over soil:streams
ui/styles.soil   one palette; dark mode via prefers-color-scheme
ui/views.soil    shell, list, detail, settings
ui/main.soil     accept loop, routes, counts
```

It reads capture bundles through `../scribe/lib/capture.soil` and writes
through `../scribe/lib/write.soil`. It does not own either.

## Routes

```text
GET  /                          list, search, date filter, hidden toggle
GET  /c/<source>/<date>/<id>    summary, actions, speakers, transcript
GET  /settings                  vocabulary
POST /speaker                   set a speaker name
POST /swap                      swap two speakers
POST /queue                     clean | voicefix | restyle | relearn
POST /vocabulary                save vocabulary
```

## Tests

```sh
SOIL=/path/to/soil ./tests/run.sh
```

Nine tests run against a disposable VFS: URL decoding, date ranges, title
derivation, word/label helpers, view shape, bundle round-trip, speaker writes,
queue de-duplication, and prefix handling.

## State

The Soil sources are verified by direct rendering and by the tests. Serving
them over a real socket depends on a host that provides `soil:streams` and
`soil:vfs-mut`; the Rust `src/` and `web/` in this directory are the previous
implementation and are scheduled for deletion once ingest is ported.
