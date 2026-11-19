# Scribe Spore

Consumes capture events and writes the durable bundle: VFS files, the
`fungos.transcript/capture@1` record, and an index refresh.

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
`fungos.transcript/capture@1`, holding content IDs, counts, bundle path,
enrichment status, and the model that produced the summary.

If inference is unavailable, the transcript is still stored and the record is
`stored_enrichment_pending`; rerun enrichment later without losing the capture.
`actions.json` is a convenience view; structured trigger records and JARVIS
routing are the next slice.

After writing, Scribe calls `soil:index.refresh()` and publishes:

- `scribe.capture.stored.v1` with the record key and content IDs.
- `scribe.capture.enriched.v1` with summary/action content IDs, model,
  duration, segment count and speaker count.
- one `scribe.action_item.v1` per extracted action, with a stable
  `action:<source>:<capture>:<index>` id and the action's quote/owner/kind.

The action events are proposals. Scribe does not create marbles or tasks and
does not execute anything. Elmers/JARVIS decide whether an item deserves a
workflow or a marble.

## Grants

`soil:vfs` (stat), `soil:vfs-mut` (write/mkdir), `soil:index` (refresh),
`soil:records` (upsert), `soil:unibus` (poll/publish), `soil:inference`
(summaries and action extraction), `soil:time` (tick), plus the WASI std
runtime. No socket, process, secret, raw filesystem, or wall clock.

## Build

```sh
cargo test
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/scribe_spore.wasm module.wasm
```

The default model alias is `scribe/summarize`. Map it in the host/Bifrost catalog
to the 9B for routine captures and add `scribe/deep` for the 65B escalation.

The pure event/path/count/transcript mapping is unit-tested. End-to-end execution needs a
host composition that links VFS, VFS mutation, index, records, and Unibus;
`soil-dev-host` currently links records and Unibus but not VFS.
