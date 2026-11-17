# deadwax

The controller for the household playhead. One GPUI window over the owned
library and the queue that is playing it.

```text
┌ deadwax   the household playhead   [● library] [● synesthesia]     ● playing ┐
│ /  search the library                          │      ◯  (a record)       │
│ ▌ Rolling in the Deep        Adele   [cid] 3:34 │  Rollin…                 │
│   In Bloom                   Nirvana [cid] 3:12 │  Adele · 21              │
│   Smells Like Teen Spirit    Nirvana [cid] 2:54 │  ────────●────── 2:41    │
│                                                 │  ◀◀   ❚❚   ▶▶    vol 80% │
│                                                 │  queue · 3               │
│                                                 │  01 Windowlicker         │
└ / search   enter play   a enqueue   space toggle   n/p skip   s source   q ┘
```

## What it owns, and what it does not

The Spore owns the view and the interaction. The host owns the audio device, the
queue writer, the Spotify session, and the Unibus client.

| It can | It cannot |
|---|---|
| search the deadwax catalog, play/queue a result | read a library file or a path |
| show the queue and playhead the owner publishes | decode, resample, or touch a sample |
| show every active concept and switch the desk's focus | decide focus without the arbiter |
| send transport commands and select a source | hold a Spotify token or an endpoint |
| search Spotify through the owner | open a socket or reach a peer |

Four imports make that split real:

- `soil:playback/player` — `sources`, `snapshot`, `send`, `search` (the focused session)
- `soil:playback/desk` — `sessions`, `focused`, `focus` (every active concept)
- `soil:playback/events` — `attached`, `poll`
- `soil:http` — one bounded `GET` against `deadwax serve`

## One desk, several concepts

Deadwax does not have to be the only player. The header is a strip of sessions:
`library` (deadwax), `spotify`, and any studio transport
(`synesthesia/arrange`, `preview`) that is on the same desk. One is **focused**;
play/pause/skip/back and seek always go to it. Clicking a session asks the desk
arbiter to focus it.

Several sessions can play at once — the host mixes — but a global verb is never
broadcast and never reinterpreted. A studio transport that has no `next track`
is still focusable; `next` fails with `unsupported` and the UI can grey it from
the capability the desk reports. A background queue cannot steal focus from a
record someone deliberately started; only a user claim does.

The arbiter is one process per desk (`canvas-winit --arbiter` on the node with
the speakers). It is arbitration, not distributed consensus: two arbiters are a
deployment bug, not a split to resolve.

Focus is not audibility. Deadwax and Raspotify play *through* the studio's audio
host (`soil:audio`); while Synesthesia holds the device for a recording session,
a player can be focused and still be locked out. It reports `blocked`, this
window says "the studio has the speakers", and the queue is left alone. See
Soil's `docs/soil-audio-studio.md`.

## Identity: CID first

`deadwax serve` now gives every indexed master a **CIDv1** (raw, sha2-256). A local
queue entry carries that CID and nothing else: not a path, not a URL, not a row
id. The provider is the host's choice — the workspace's `soil:vfs` copy, the
local `GET /content/:cid` range endpoint, or `unibus-ipfs` — and the Spore is
indifferent to all three.

That is what makes the same window work on a laptop and on a Pi: the queue names
bytes, and the owner resolves them on the machine with the speakers.

## Spotify

`apparent source` is one `player.search` call, not a second HTTP client in the
guest. The host adds its credential, so a Spore that can search Spotify still
cannot read a token. A Spotify queue entry is a `spotify:track:…` URI.

## The Unibus bridge

The wire contract lives in unibus: `audio.playback.state.v1`,
`audio.playback.event.v1`, `audio.playback.command.v1`. Soil's
`soil-playback-host` bridges them into the interfaces above, field for field;
the same kinds are what `soil:unibus` exposes when a Spore wants the raw bus. This Spore does not need that grant: the typed edge is enough. A second
controller sees the same queue because it reads the same owner, not because
controllers talk to each other.

```text
Spotify / MPD / local ──► owner ──► audio.playback.* ──► host ──► events.poll
                              ▲                                      │
                              └──────── player.send ◄───────────────┘
```

## Build

```sh
cd spores/deadwax
cargo build --release --target wasm32-wasip2
wasm-tools strip target/wasm32-wasip2/release/spore_deadwax.wasm -o module.wasm
wasm-tools validate module.wasm
```

`cargo test` here runs the view's parser and progress tests; they use catalog
fixtures copied from live responses, which is how the no-CID (discogs) case
stays visible instead of blanking the list.

## Host requirement

`soil:playback` is a canonical Soil contract (`wit/soil-playback.wit`, vendored
here from `fungOS-linux/soil`). The host is Soil itself:

- `crates/soil-webgpu` links `soil:playback/player` and `events` for a GPUI
  Spore; `HostSetup::playback` carries the owner.
- `crates/soil-playback-host` implements the owner over the
  `audio.playback.*` Unibus kinds through `soil:unibus`.
- `crates/canvas-winit` attaches it with `--unibus-config
  <ClientConfig.json>` (plus optional `--unibus-hub` overrides).

```sh
canvas-winit spores/deadwax/module.wasm --unibus-config client.json
```

Without a config the window still opens and the library still searches;
transport returns `unavailable` instead of pretending there is a queue. A host
that does not carry the package refuses this Spore by name, not half-blind —
the same rule `spores/elmers-ui` follows for its `soil:http` import.

## GPUI Kit

The view is built from the kit's styled layer (`gpui-component`, from
`fungOS-linux/gpui-kit`): `Root` owns the window and overlays, `Theme` owns the
palette, `Tag` draws the identity chips. The rest is the same shape as
[`spores/terminal-gpui`](../terminal-gpui): an offscreen `HeadlessAppContext`, a
frame readback, and `soil:present/surface.present`.
