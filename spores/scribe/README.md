# Scribe Spore

Consumes capture events and writes the durable bundle: VFS files, the
`fungos.transcript/capture@1` record, and an index refresh.

## Input

`omi.capture.v1` on Unibus, or `ingest --event JSON` for tests.

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
scribe watch [--hold-ms N] [--once]
scribe ingest --event '<json>'
scribe snapshot
```

`watch` polls `omi.capture.v1`, processes each frame, and advances the Unibus
cursor. `--once` processes one page and exits; this is the test/dev path.

## Output

```
captures/<source>/<date>/<capture_id>/
  transcript.json
  summary.md
  actions.json
```

The record is `capture/<source>:<capture_id>`, schema
`fungos.transcript/capture@1`, holding content IDs, counts, bundle path, and
status. `actions.json` is a convenience view; structured trigger records and
JARVIS routing are the next slice.

After writing, Scribe calls `soil:index.refresh()` and publishes
`scribe.capture.stored.v1` with the record key and content IDs.

## Grants

`soil:vfs` (stat), `soil:vfs-mut` (write/mkdir), `soil:index` (refresh),
`soil:records` (upsert), `soil:unibus` (poll/publish), `soil:time` (tick), plus
the WASI std runtime. No socket, process, secret, raw filesystem, or wall clock.

## Build

```sh
cargo test
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/scribe_spore.wasm module.wasm
```

The pure event/path/count mapping is unit-tested. End-to-end execution needs a
host composition that links VFS, VFS mutation, index, records, and Unibus;
`soil-dev-host` currently links records and Unibus but not VFS.
