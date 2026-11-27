# Scribe Spore

Consumes capture events and writes the durable bundle: VFS files, the
`fungos.transcript/capture@1` record, and an index refresh. It also serves the
browser view over those bundles.

Scribe is the reference Soil port: one Spore, two surfaces (`cli` and `web`),
one shared domain library, and no duplicated capture logic.

## Layout

```text
spore.soil          Spore v0 manifest (source-distributed)
spore.lock          package identity and file digests
wit/world.wit       the cli and web worlds
source/lib/         capture.soil (read) and write.soil (mutate)
source/ui/          http, styles, views, main
tests/              nine Soil tests and a disposable-VFS runner
src/, Cargo.toml    the previous Rust module (legacy, until ingest is ported)
spore.toml          the previous manifest (superseded by spore.soil)
```

`source/lib/capture.soil` reads bundles and never writes.
`source/lib/write.soil` writes and never parses. A spore that only presents
needs `soil:vfs`; an editing spore adds `soil:vfs-mut`. That split is what lets
ingest and the UI share one reader without sharing grants.

`module.wasm` in this tree is the Rust build. It is not the implementation
`spore.soil` describes, and it is deleted with `src/` when the ingest port
lands.

## Reading and writing

```text
GET  /                          list, search, date filter, hidden toggle
GET  /c/<source>/<date>/<id>    summary, actions, speakers, transcript
GET  /settings                  vocabulary
POST /speaker                   set a speaker name
POST /swap                      swap two speakers
POST /queue                     clean | voicefix | restyle | relearn
POST /vocabulary                save vocabulary
```

Pages are server-rendered from `el`/`attr`/`txt` in `source/ui/`. There is no
client JavaScript. HTTP is parsed in Soil (`source/ui/http.soil`), which keeps
the host to a byte pump.

## Input

`scribe.capture.v1` on Unibus, or `ingest --event JSON` for tests. The event may
carry `summary_md`/`actions`; if they are absent, Scribe generates them through
`soil:inference`.

```json
{
  "capture_id": "omi-123",
  "source": "omi",
  "date": "2026-11-19",
  "started_at_ms": 1763500000000,
  "ended_at_ms": 1763503600000,
  "transcript": {
    "text": "full diarized text",
    "words": [{"word": "hello", "start": 0.1, "end": 0.5, "speaker": 1}]
  },
  "summary_md": "# Summary\n...",
  "actions": [
    {"kind": "action", "title": "Send the proposal", "quote": "...", "owner": "avery"}
  ],
  "audio_content_id": "blake3:..."
}
```

`transcript` may also be a JSON string. `transcript_json` is accepted for
compatibility. `body` wrappers are unwrapped.

## Commands

```text
scribe watch [--hold-ms N] [--once] [--model ALIAS] [--no-enrich]
scribe ingest --event '<json>' [--model ALIAS] [--no-enrich]
scribe receive [--once] [--model ALIAS] [--no-enrich]
scribe snapshot
```

`receive` is the host-owned WSS transport: it accepts one bundle, writes the
audio and event through VFS, then runs the same ingest/enrichment path.
`watch` polls `omi.capture.v1`, processes each frame, and advances the Unibus
cursor. `--once` processes one page and exits; this is the test/dev path.

## Output

```
captures/<source>/<date>/<capture_id>/
  transcript.json
  summary.md          # display: Scribe's when available, else Omi's
  summary.omi.md      # Omi's supplied summary, preserved when present
  summary.scribe.md   # Scribe's generated summary, when enrichment ran
  summary.json        # provenance: display source + both content ids
  actions.json        # tasks, feature requests, decisions, commitments
  questions.json      # things to resolve, not tasks
  calendar.json       # calendar candidates, not sent invites
```

The record is `capture/<source>:<capture_id>`, schema
`fungos.transcript/capture@1`, holding content IDs, counts, bundle path,
enrichment status, and the model that produced the summary.

If inference is unavailable, the transcript is still stored and the record is
`stored_enrichment_pending`; rerun enrichment later without losing the capture.
Scribe keeps Omi's summary and its own summary side by side. `summary.md` is
the display copy: Scribe's when enrichment succeeded, otherwise Omi's. The
Omi text is never overwritten, so a model regression is reversible.
`actions.json`, `questions.json`, and `calendar.json` are separate proposals.
Omi's supplied copies are kept as `*.omi.json` when present.

Actions are things someone should do. Questions are things to resolve, with an
owner/direction and a blocking flag. Calendar entries are candidates for the
calendar inbox: `status` is `hold`, `tentative`, or `confirmed`, and `attendees`
distinguish people present in the capture from people named as invitees. Scribe
never sends an invite and never invents an email; unresolved attendees are left
for the inbox to contact.

After writing, Scribe calls `soil:index.refresh()` and publishes:

- `scribe.capture.stored.v1` with the record key and content IDs.
- `scribe.capture.enriched.v1` with summary/action content IDs, model,
  duration, segment count and speaker count.
- one `scribe.action_item.v1` per extracted action, with a stable
  `action:<source>:<capture>:<index>` id and the action's quote/owner/kind.
- one `scribe.question.v1` per open question.
- one `scribe.calendar_candidate.v1` per calendar candidate.

These events are proposals. Scribe does not create marbles, tasks, calendar
events, or ICS, and does not execute anything. Elmers/JARVIS decide whether an
item deserves a workflow or a marble; the calendar inbox resolves attendees,
time zones, recurrence, and ICS status.

## Grants

`soil:vfs` (read), `soil:vfs-mut` (write), `soil:streams` (UI sessions and
capture receive), `soil:index` (refresh), `soil:records` (upsert),
`soil:unibus` (poll/publish), `soil:inference` (summaries and action
extraction), `soil:time` (tick), plus the WASI std runtime. No socket, process,
secret, raw filesystem, or wall clock.

## Tests

```sh
SOIL=/path/to/soil ./tests/run.sh
```

The runner builds a disposable VFS and records book and runs every `test/*`
function: URL decoding, date ranges, title derivation, word/label helpers, view
shape, bundle round-trip, speaker writes, queue de-duplication, and prefix
handling.

## Build

The legacy module:

```sh
cargo test
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/scribe_spore.wasm module.wasm
```

The manifest and lock:

```sh
spores/../soil/scripts/spore-pack.sh lock spores/scribe
```

The default model alias is `scribe/summarize`. Map it in the host/Bifrost catalog
to the 9B for routine captures and add `scribe/deep` for the 65B escalation.

The pure event/path/count/transcript mapping is unit-tested. End-to-end execution
needs a host composition that links VFS, VFS mutation, index, records, and
Unibus; the previous `soil-dev-host` links records and Unibus but not VFS.

## Port

`docs/scribe-soil-port.md` records the boundary, the host gap, the ingest plan,
and why `omi-rs` is now only useful as a decoder.
