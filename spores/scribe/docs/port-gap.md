# Scribe: Soil port gap

Status: **ingest logic ported (pure half); Unibus loop next**, 2026-11-23.
`src/lib.rs` (3103 lines) is still the shipped module. The Rust tree is the
reference for behaviour, not the target.

## Where the port stands

```text
source/lib/capture.soil    398   read bundles (soil:vfs only)          ported
source/lib/write.soil      173   write bundles, speakers, queues       ported
source/lib/ingest.soil     ~230  event normalization, transcript text,
                                 chunking, title/emoji, provenance      NEW
source/ui/*                738   http, styles, views, main             ported
tests/soil/                 ~170 6 UI tests + 11 ingest tests          passing
src/lib.rs                3103   ingest, unibus, enrichment, audio     to port
```

Test gate (needs the fork runtime — see "Runtime pin" below):

```sh
SOIL_LANG=~/src/fungOS/soil/target/release/soil-lang tests/run.sh
# ok test/url-decode … ok test/event  (17 total)
```

## Rust function map → port plan

| Rust | what | Soil plan |
| --- | --- | --- |
| `parse_event`, `sanitize`, `normalize_date`, `summary_title`, `short_title`, `emoji_for`, `speaker_count`, `segment_count` | event normalization | **done** in `ingest.soil` |
| `transcript_text`, `speaker_label`, `split_chunks` | model input | **done** in `ingest.soil` (segments > text > words, matching Rust) |
| `summary_source` logic inline in `process_event` | provenance (`scribe`/`omi`/`empty`) | **done** as `ingest/summary-source` |
| `write_file`, `mkdir_recursive`, `process_event` (bundle half) | VFS bundle + records + index | P2: extend `write.soil`; `mkdir` per component, write transcript/summary/actions/questions/calendar, upsert `capture@1`, call `soil:index` |
| `watch` | Unibus poll loop | P3: ~30 lines in Soil (`soil:unibus/poll` + cursor + `process-event`), publish `scribe.capture.stored.v1` |
| `enrich_capture` | prompts, chunk loop, inference, JSON parse/merge | P4: `soil:inference/complete` per chunk, `json-decode` with the same tolerance for fenced/prose JSON |
| `learn_manual`, `voiceprint*`, `voicefix*`, `restyle*`, `clean*`, `merge*` | audio + embeddings | **P6 / stay in Rust**: needs byte buffers, float math (cosine), WAV PCM splicing. Do not block the port on these. |
| `receive`, `receive_one`, `stream_path` | host-owned WSS bundle | **do not port**: replace with the Unibus edge (see below) |

## Runtime pin (found while porting)

The installed `~/.local/share/soil/bin/soil-lang` is **older than the spore's
Soil code**: it lacks `string-split`, `lambda`, `min`/`max`, and `defconst`,
so `tests/run.sh` fails on `test/words-and-labels` before reaching ingest. The
fork tree (`~/src/fungOS/soil`, branch `pi/soil-process`) has all of them and
builds a runtime the whole suite passes on. Two consequences:

1. `tests/run.sh` should default to a **pinned runtime path**, not `PATH`
   discovery, and the spore should record which runtime it was tested against.
2. `ingest.soil` deliberately avoids `defconst`, `lambda`, `min`/`max`,
   `string-split` and any char-code primitive, so it runs on both trees. Keep
   new port code to the intersection until the runtime is pinned.

Also: the fork runtime prints `granted called: …` on every host call and
`allow_process is false` once; harmless, noisy, and a sign the debug line
should be behind a flag.

## Unibus loop (P3) — the shape

The WIT is already vendored (`wit/deps/soil-unibus.wit`):

```wit
poll: func(kinds: list<string>, cursor: option<string>, limit: u32) -> result<page, error>
publish: func(kind: string, body: list<u8>, ttl-ms: option<u64>) -> result<receipt, error>
```

Soil version of `watch`:

```soil
(defun scribe/watch (cursor seen stored failed)
  (let ((page (soil:unibus/poll (list "omi.capture.v1") cursor 32)))
    … process each frame with ingest/event + write/bundle …
    (if (nil? (field page :next)) (counts …)
        (scribe/watch (field page :next) (+ seen n) (+ stored k) failed))))
```

Rules to keep:

- **Cursor advances only after a frame is durably stored** (Rust does this; a
  crash mid-batch must replay, not skip).
- **A bad frame is counted, not fatal** (`failed`), so one malformed body does
  not stall the loop.
- **`publish` is best-effort after the write**, never before.
- `--once` for tests, `--hold-ms` for the loop; both are argument plumbing.

## The edge question (soil:streams vs Unibus)

Today `receive` is a host-owned WSS session: the host accepts one bundle, writes
the audio and event through VFS, then runs ingest. That makes the *transport*
part of the spore's surface even though the spore cannot own a socket
(`transport = ["socket", "quic", "iroh"]` is denied).

Recommended edge:

```text
Omi app → host bridge (owns WSS/QUIC) → Unibus frame omi.capture.v1 → scribe
```

- One transport, replayable (retained frames + cursor), no duplex session in the
  spore, and the loop above already handles it.
- Keep `receive` only as a **host-side adapter** that republishes on the bus;
  delete it from the spore when the bridge lands.
- Audio stays a VFS object: the frame carries `audio_content_id` and the bytes
  arrive through the host's capture path, not through the spore.

So: port the Unibus loop, do not port `receive`, and treat `soil:streams` as the
UI transport only (the web surface), not as the ingest edge.

## Transcripts in VFS (already the right shape)

```text
captures/<source>/<date>/<capture_id>/
  transcript.json  summary.md  summary.omi.md  summary.scribe.md
  summary.json  actions.json  questions.json  calendar.json
  capture.json     # title, emoji, summary_source, times
```

Rules worth keeping through the port:

- **Omi's text is never overwritten**; Scribe's summary is a sibling file and
  `summary.json` says which one is displayed.
- Enrichment failure is a status (`stored_enrichment_pending`), not a lost
  capture — rerun enrichment later.
- The bundle is the durable unit; the record indexes it by content id.

## Memories and habits (new; not in the Rust tree)

The point of keeping transcripts as immutable bundles is that everything else is
*derived* and can be recomputed. Two derived layers, both in VFS + records:

```text
memories/<subject>/<memory-id>.json
  kind:        fact | preference | commitment | relationship
  subject:     person id, project, or topic
  text:        one sentence, imperative-free ("Avery prefers X" not "remember X")
  confidence:  0..1
  sources:     [capture ids]        # provenance, mandatory
  first_seen_ms, last_seen_ms
  status:      proposed | confirmed | rejected | superseded

habits/<habit-id>.json
  pattern:     "standup runs long on Mondays" (text) + machine key when known
  cadence:     daily | weekly | per-capture
  evidence:    [capture ids], count, span
  confidence
  last_seen_ms
```

Pipeline (all offline, never at read time):

```text
capture stored ──▶ extract candidates (soil:inference, JSON schema)
               ──▶ write <bundle>/memories.proposed.json
consolidate (nightly) ──▶ dedupe by text/embedding, bump last_seen,
                          promote to memories/<subject>/, record memory/<id>
habit pass (weekly)   ──▶ scan the capture index, not the bundles
```

Design rules:

- **Provenance is mandatory.** A memory without `sources` is unrepresentable;
  the UI shows "from 3 captures" and can drop it if they are deleted.
- **Derived, never authoritative.** Deleting a bundle must invalidate memories
  that cite it (the consolidate pass checks existence).
- **Proposals first.** Inference writes to `proposed`; confirmation is a human
  or a threshold, so a model regression cannot corrupt the durable layer.
- **Habits read the index, not transcripts.** A habit is a pattern over many
  captures; scanning bundles for it is the same OOM trap Doctor hit.
- **No memory is read at capture time.** Ingest must work with inference down.

## Phases

```text
P1  ingest pure half                      DONE (this change)
P2  bundle writer + capture@1 + index     next; reuses write.soil
P3  Unibus watch loop                     ~30 lines + argument plumbing
P4  enrichment (prompts, chunking, JSON)  needs complete + tolerant parse
P5  memories/habits                       after P2 (needs the index)
P6  audio tools                           keep in Rust; needs bytes + floats
```

P2–P4 remove `src/lib.rs` except audio. P6 can be deferred indefinitely without
blocking the port: it is the only part that genuinely needs the object model to
grow byte buffers and floating point, which is the same gap `SELF_HOSTING.md`
records for the compiler.
