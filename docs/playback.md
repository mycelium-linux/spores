# `soil:playback` — the guest side of the playhead

A Spore that shows or drives music does not open an audio device, hold a
provider token, or speak Unibus. It asks the host for the household playhead:
what is playing, what is queued, and a vocabulary for moving it.

`spores/deadwax` is the first consumer. The package is vendored at
[`spores/deadwax/wit/deps/soil-playback/package.wit`](../spores/deadwax/wit/deps/soil-playback/package.wit).

## Coexistence: one desk, several concepts

`deadwax`, `spotify`, and a studio project (`synesthesia`) are not competitors
for one playhead. Each is a **session** — an `(owner, session)` pair — on one
desk. Several can play at once; exactly one is **focused**, and play/pause/skip/
back always go to it.

```wit
interface desk {
  sessions: func() -> list<session>;
  focused: func() -> option<session-ref>;
  focus: func(target: session-ref) -> result<_, error>;
}
```

- A session declares what it can honour (`can-next`, `can-previous`, …). A DAW
  with no `next track` is still focusable; `next` fails with `unsupported`
  rather than being reinterpreted, and the UI greys it out.
- Focus is decided by one arbiter per desk (`audio.playback.focus.v1`): a user
  claim wins and is held; an automatic start wins only when the desk is free or
  the focused session is idle; a quiet session falls out and the most recently
  playing one takes over.
- Focus is not audibility. The audio host (Synesthesia) can hold the device for
  a recording session; a player may be focused and still be locked out. It keeps
  its queue and reports `blocked`, which this controller shows as a notice, not
  an error. See Soil's `docs/soil-audio-studio.md`.

## Why this is separate from `soil:media`

| | `soil:media` | `soil:playback` |
|---|---|---|
| Scope | one host-decoded source | the household queue and transport |
| Names | host-reviewed source id | a CID (owned master) or a Spotify URI |
| Owner | the player of that one source | the node with the speakers and the account |
| Events | `next` on a playback resource | `events.poll`, fed by Unibus |

A playback owner may use `soil:media` internally to play what the queue points
at. A controller never imports it: decoding is not its grant.

## The contract

```wit
interface player {
  sources: func() -> list<capability>;
  snapshot: func() -> result<state, error>;      // the focused session
  send: func(command: command) -> result<u64, error>;
  search: func(source: source, query: string, limit: u32) -> result<list<track>, error>;
}

interface desk {
  sessions: func() -> list<session>;
  focused: func() -> option<session-ref>;
  focus: func(target: session-ref) -> result<_, error>;
}

interface events {
  attached: func() -> bool;
  poll: func(timeout-ms: u32) -> option<event>;
}
```

- **`state` is authoritative.** `revision` is monotonic per session. An
  event is a hint that the snapshot changed; a consumer that misses one reads
  `state` again rather than replaying a delta.
- **`send` is a request, not a guarantee.** The owner answers with the revision
  it produced. A command written against a stale revision fails with `stale`
  instead of landing on a queue it was not written for.
- **`search` keeps credentials out of the guest.** The owner already has the
  Spotify account, so it is the only party that can search it without putting a
token in the component.
- **`poll` always waits.** A host with no owner attached returns `attached() =
  false` and still honours the timeout, so a live Spore can use it as a frame
  clock. A one-shot capture host returns `interactive() = false` through
  `soil:present/input` and the Spore exits after one frame.

## What a host must do

Soil already does this: `crates/soil-playback-host` is the owner adapter and
`crates/soil-webgpu` links the two interfaces. The steps below are what that
adapter implements, and what a different host must reproduce.

1. Own one playback owner (MPD, a local player, Spotify Connect) and one queue.
2. Subscribe to `audio.playback.state.v1` / `audio.playback.event.v1` on
   Unibus and cache the latest snapshot.
3. Implement `snapshot` / `sources` from that cache and `send` by publishing
   `audio.playback.command.v1` with `expected_revision` and a deadline inside
   the state lease.
4. Resolve a queue identity: a CID through the workspace's `soil:vfs` copy,
   `deadwax serve`'s `GET /content/:cid`, or `unibus-ipfs`; a Spotify URI through
   the owner's session.
5. Bound the queue (200 entries), the search (`limit`), and the body of a
   catalog fetch.

The mapping is field-for-field, so no translation table should be necessary:
see unibus's `docs/playback-contract.md`.

## Identity is not location

- `local` id: CIDv1, raw codec, sha2-256, base32 lower — 59 characters starting
  `bafkrei`. `deadwax cid <file>` prints it; `/tracks/:id` carries it.
- `spotify` id: `spotify:track:<22>` or `spotify:episode:<22>`.

A path, URL, row id, or provider-internal id is rejected before it reaches a
queue. That is what lets the controller run on a machine that has neither the
files nor the Spotify session.

## Status

Wired in Soil. `wit/soil-playback.wit` is canonical, `soil-playback-host`
bridges `audio.playback.*` through `soil:unibus`'s transport, `soil-webgpu`
links both interfaces, and `canvas-winit` attaches an owner from
`--unibus-config`. This Spore is the first consumer; the `deadwax` side — CID
per master and the `GET /content/:cid` resolver — is implemented and tested.
