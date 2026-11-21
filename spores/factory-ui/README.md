# factory-ui

The software-factory loop, read as a page — server-rendered, no client state, no
index of its own. Every byte comes from `soil:vfs`; every comment goes back into
the same root through `soil:vfs-mut`.

## What it reads

The control plane (edgerunner) writes a transcript as a **head ref plus sealed
segment objects**:

```text
transcripts/<session>/head.json          mutable ref: segment list, content ids,
                                         plus events that have not sealed
transcripts/<session>/seg-<a>-<b>.jsonl  immutable, one event per line
transcripts/<session>/comments/<id>.json immutable once written
```

`head.json` carries the content id of every segment it names, so a reader can
verify that the bytes it hydrated are the bytes the head meant. The tail is the
only mutable transcript and it only appends, so a line quoted from it keeps
meaning the same thing after the segment seals.

## What a comment is

A comment is anchored to **bytes**, not to a line number:

```json
{
  "anchor": {
    "path": "transcripts/sess_x/seg-000001-000064.jsonl",
    "content_id": "blake3:…",
    "seq": 12,
    "start": 340,
    "end": 372,
    "quote": "the reviewed sentence"
  },
  "body": "…", "author": "…", "parent": null
}
```

That is the delta property. A rename cannot move the comment (the content id is
unchanged). A rewrite cannot silently re-point it: the page compares the
anchor's content id against the segment the head names now and says *"written
against an earlier revision"* instead of attaching the comment to whatever
occupies those bytes today. The quote is kept as evidence either way.

The guest does not trust the browser for offsets. The page sends the event's
`seq` and the selected text; the guest finds the event's line in the segment
file and locates the quote in those bytes. A quote that is not there is refused
(`400`), because a comment that was never anchored is worse than one that
failed.

## Routes

| method | path | |
|---|---|---|
| GET | `/` | sessions in the root, newest first |
| GET | `/s/<session>` | transcript, comments, composer |
| GET | `/api/comment?session=<session>` | that session's comments as JSON |
| POST | `/s/<session>/comment` | write one comment (`body`, `seq`, `quote`) |
| GET | `/healthz` | |

## Run

```sh
# build (the guest is a component, not a binary)
RUSTC_WRAPPER= /Users/ajmwagar/.cargo/bin/cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/factory_ui.wasm module.wasm

# serve the same root the control plane writes
soil spore run . --surface http --listen 127.0.0.1:8091 \
  --vfs-root ~/factory-vfs
```

The host's `--vfs-root` is the grant: the Spore can read the transcript objects
and write a comment beside them, and cannot see a host path, the network, the
clock it did not ask for, or any secret.

## Not done, and named

- **Identity.** Comments are attributed to a constant until the page sits behind
  the SSO ingress that gives each person their own actor (`bridge-identity.md`).
- **Replies in the UI.** `parent` is written and rendered, but the composer only
  starts a thread; replying is the next form.
- **Segments are read whole.** A transcript of thousands of events costs a full
  read per page. The head already gives the ranges to page over; nothing here
  needs it yet.
