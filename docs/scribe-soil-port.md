# Scribe: the first solid spore

Status: UI sliced and verified; ingest pending.

Scribe is the reference port for moving a Spore off hand-written Rust and into
Soil. The order is deliberate:

1. **UI first.** Replace the 1,038-line Rust HTTP handler and its 673-line
   `ui.html` with Soil forms. No client JavaScript.
2. **Domain library.** Put capture-bundle reading in a library both the UI and
   the ingest side can call.
3. **Ingest second.** Port the 3,103-line `scribe/src/lib.rs` watcher, receiver,
   and snapshot pipeline onto the same library.

The measure of success is less total code and more shared code, not a line-for-line
translation.

## What landed

```text
spores/scribe/spore.soil          Spore v0 manifest, source-distributed
spores/scribe/spore.lock          package identity and file digests
spores/scribe/wit/world.wit       the cli world and the web world
spores/scribe/source/lib/capture.soil   read-only domain layer over soil:vfs
spores/scribe/source/lib/write.soil     every soil:vfs-mut write
spores/scribe/source/ui/http.soil       request line, path, query, headers, body
spores/scribe/source/ui/styles.soil     one palette, dark via media query
spores/scribe/source/ui/views.soil      shell, list, detail, settings
spores/scribe/source/ui/main.soil       accept loop and routes
spores/scribe/tests/                    nine tests plus a disposable-VFS runner
```

`capture.soil` owns paging, chunked reads, date keys, titles, transcripts,
speakers, filtering, cards, and search. `write.soil` owns the JSON writers,
speaker renames/swaps, the clean/voicefix/restyle/learn queues, and vocabulary.
No capture logic is duplicated between the UI and ingest.

### Boundary

`capture.soil` never writes. `write.soil` never parses. That split is what lets
ingest and presentation share the reader without either one taking on the
other's grants. A spore that only presents needs `soil:vfs`; only a spore that
edits needs `soil:vfs-mut`.

### One spore, two surfaces

The port merges the former `scribe` and `scribe-ui` packages. Spore v0 gives a
package several surfaces, so ingest is `cli` and the browser view is `web` over
the same `source/` tree. That is what makes the shared library real: there is no
second package to vendor `capture.soil` into, and no way for the two halves to
drift onto different readers.

```lisp
(spore
  (schema 1)
  (kind "spore")
  (id "dev.fungos.scribe")
  (world "fungos:scribe/scribe@0.1.0")
  (wit "wit/")
  (distribution "source")
  (source "source/")
  (surfaces
    (surface "cli" (world "fungos:scribe/scribe@0.1.0"))
    (surface "web" (world "fungos:scribe/scribe-web@0.1.0"))))
```

`spore.lock` covers every file in the tree, including the legacy Rust module
that is still here so the app stays deployable. The lock is regenerated when
`src/`, `spore.toml`, and `module.wasm` are deleted with the ingest port.

## UI shape

Pages are server-rendered forms from `el`/`attr`/`txt`/`render-html`:

```text
GET  /                     list + search + date filter
GET  /c/<source>/<date>/<id>   detail: summary, actions, speakers, transcript
GET  /settings             vocabulary
POST /speaker              name a speaker
POST /swap                 swap two speakers
POST /queue                clean, voicefix, restyle, relearn
POST /vocabulary           save a vocabulary
```

The old Rust API routes collapse into these pages and four POSTs. Queues keep
the same files: `scribe/clean.json`, `voiceprints/voicefix.json`,
`voiceprints/restyle.json`, `voiceprints/learn.json`, `scribe/merge.json`.

HTTP is parsed in Soil rather than the host. This keeps the host to a byte pump
and makes the request shape visible to the spore. When a second spore needs it,
`ui/http.soil` is a candidate for `stdlib/http.soil`; until then it stays local
rather than becoming a shared API with one user.

## Running the tests

```sh
cd spores/scribe
SOIL=/path/to/soil ./tests/run.sh
```

The runner builds a disposable VFS and records book, loads the prelude, UI,
both bindings, both libraries, all four UI files, and the test file, then calls
each `test/*`. It needs the `soil-lang` binary from the Soil tree that owns
`crates/soil-lang`; set `SOIL_LANG` to use a prebuilt one.

Rendering a page without a socket is a single call:

```sh
soil-lang --prelude <soil>/crates/soil-lang/stdlib/prelude.soil \
  --workspace-root /tmp/scribe-vfs --records-root /tmp/scribe-records --book scribe \
  <bindings>/soil-vfs.soil <bindings>/soil-vfs-mut.soil \
  source/lib/capture.soil source/lib/write.soil \
  source/ui/http.soil source/ui/styles.soil source/ui/views.soil source/ui/main.soil \
  --call scribe-ui/render-detail probe.soil
```

That is how the list (6 KB), detail (7.5 KB), and settings (5.8 KB) pages were
checked, and how the speaker, queue, and vocabulary writes were verified.

## Host gap

The UI is proven by rendering and by its mutation tests. It is not yet served
over a real socket in this tree. `soil spore run <dir> --listen ADDR` is the
intended entry point for a source Spore, but the installed `soil` binary
predates the `spore.soil` manifest and still looks for `spore.toml`. Two host
options exist and neither is finished:

- `soil-serve` in the Soil tree owns a TCP listener and one `soil:streams`
  session per connection, but it hardcodes a harness path and a
  `server/main` + `server/head` contract.
- The newer Soil tree (`core/`, `compiler/`, `spore.soil`) has the same UI
  primitives but no generated `soil:vfs-mut` or `soil:streams` bindings yet,
  and its installed `soil` binary predates the `spore.soil` manifest.

The WIT for both capabilities exists in Loam (`crates/loam/wit/vfs` and
`crates/loam/wit/stream`). Generating those bindings and running
`soil spore run --listen` is the unblock. Nothing in the scribe sources needs
to change for it; only the load list and manifest do.

## Next

1. Generate `soil-vfs-mut` and `soil-streams` bindings in the current Soil tree,
   then serve `main.soil` over a socket and exercise a POST round-trip.
2. Port ingest: watcher, receiver, snapshot, and bundle writer onto
   `capture.soil` + `write.soil`. Keep `omi-decoder` and inference as host calls.
3. Point STT at the DGX Spark through `fpl/transcribe` (`fpl/stt` alias) and
   confirm `scribe/summarize` and `scribe/deep` route there.
4. Delete `src/`, `spore.toml`, and `module.wasm`, then re-lock:
   `../soil/scripts/spore-pack.sh lock spores/scribe`.
5. Measure the delta and move the remaining spores onto the v0 layout.

## omi-rs

`omi-rs` exists to speak to the pendant: BLE transport, Opus decode, a cloud STT
call, and memory. Scribe needs none of that once the pendant writes an
`omi.capture.v1` bundle:

- The decoder is the only piece on the critical path, and it stays Rust.
- STT now runs on the DGX Spark, not in the spore.
- The rest is a second implementation of a pipeline nothing in this tree calls.

Keep it only if the pendant is still a supported capture source. If it is, it
should shrink to transport plus a bundle writer, and it should call the same
capture library rather than growing a parallel one.
