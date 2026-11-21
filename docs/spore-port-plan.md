# Spore port and retirement plan

**Status:** proposed · **Scope:** `fungOS/spores` at `a96bc80` · **Companion:** Soil's
`docs/soil-lisp-roadmap.md`

Spores are not all the same kind of thing. Some are domain libraries, some are
capability fixtures, some are host shims, and some are entire applications whose
only interesting code is a Rust parser or a GPU renderer. Treating "port to
Soil" as one bucket would either keep dead fixtures forever or rewrite a
Markdown parser in Lisp for no reason.

This plan classifies every Spore by the layer it owns, then says what to delete,
what to leave alone, and what to move into Soil. The target is not "no Rust".
The target is:

1. domain logic (targets, scenes, queues, capture schemas) is Soil;
2. host policy and secret handling stay in the host;
3. rendering, codecs and device transports stay in Rust until Soil has a
   better primitive;
4. fixtures and duplicate UIs stop shipping.

The old nested `soil/spores-public` checkout is behind this repository. This
plan is about `fungOS/spores`; where the two disagree, this tree wins.

## Decision rules

A Spore is a **port candidate** when most of its source is orchestration:
HTTP/records/VFS calls, JSON shaping, state machines, schedules, and small
domain rules. Those are exactly the things Soil can express without losing
performance.

A Spore **stays in Rust/Wasm** when its value is a parser, codec, GPU renderer,
device transport, or sandbox/host boundary. A Soil wrapper can call it later,
but porting it would be a rewrite of a solved problem.

A Spore is a **delete candidate** when it is one of:

- a capability fixture whose test belongs in Soil's test suite;
- a duplicate view over the same data with no unique interaction;
- a probe that exists to inspect the host, not to do work;
- a package whose only consumer is itself.

Deletion means removing the package from this repo (git history keeps it).
Retirement means keeping a shared library until its last consumer moves, then
removing it in the same change that removes the consumer.

## Summary

| Disposition | Spores |
|---|---|
| Port now | `hue`, then `deadwax`, `scribe`, `spotify` |
| Port after the first gate | `weather`, `vitals`, `pulse`, `worklog`, `fleet`, `git`, `files`, `search` |
| Port with the UI layer | `launcher`, `home`, `marbles-ui`, `scribe-ui`, `lights-view` |
| Keep in Rust | `markdown`, `dfm-report`, `omi-decoder`, `pinetime`, `colmi-ring`, `terminal`, `tui`, `mush`, `rooms`, `consent`, `dragshift` |
| Keep as host/package tooling | `chrome-host`, `launcher-host`, `spore-browser-host`, `spore-serve`, `spore-catalog`, `themes`, `metrics-sdk`, `health-smoke` |
| Delete / fold | `probe-gpui`, `media-probe`, `surface-probe`, `gallery`, `chrome` (into `divebar`), `board` (into `marbles-ui`), `spore-browser` (into `launcher`) |
| Move to the private spores repo | `jarvis-brief`, `jarvis-inbox`, `jarvis-telemetry` |

## P0 — Hue, the first real port

`hue` is the right first port because it is small, useful every day, and its
boundary is already clean: the host owns the connection and the bridge key, and
the Spore owns Hue semantics.

The Soil port lands in `fungOS-linux/soil`:

- `examples/hue/hue.soil` — bridge URL construction, targets, scenes,
  brightness, RGB→xy, and Hue's in-band error decoding;
- `crates/soil-lang/bindings/soil-http.soil` — the generated binding;
- native `soil:http` in `NativeHost`, with an origin allowlist and a path
  credential under the guest handle `default`;
- REPL instructions in `examples/hue/README.md`;
- a fake bridge test for the semantics and a local-socket test for the host.

That yields the daily loop:

```lisp
(defun home () (hue/bridge "http://192.168.1.10" "default"))
(hue/on (home) (hue/group "0"))
(hue/bri (home) (hue/light "3") 180)
(hue/scene (home) (hue/group "1") "Relax")
```

Once that is stable, `lights-view` is the next consumer, not the next port:
its room/light state machine should call the same Soil library. The GPUI
renderer can stay Rust while `soil:ui` is unfinished.

**Gate:** a Soil program can turn a room on at a schedule, read bridge state,
and recover from a bridge error without a Rust change.

## P1 — the two spores worth bringing

### `deadwax`

`deadwax` is a household playhead controller, not a decoder. Its Rust source is
mostly catalog search, queue state, and transport commands. That belongs in
Soil:

- catalog search and result shaping;
- queue admission (CID versus Spotify URI);
- transport commands to the owner;
- the "what is playing" view model.

The GPUI surface can remain a Rust view until the Soil UI path exists. The
important split is that the controller logic becomes callable from a Soil
program (and from a future shell), not trapped inside one window.

### `scribe`

`scribe` is the best natural Soil application in the tree: consume a capture
event, read/write VFS, write records, call inference, refresh the index, and
publish a result. It is almost entirely orchestration.

Port `scribe` as a Soil library with a small native entry point. Keep the
`scribe-ui` web surface as-is initially; it is a presentation layer over VFS
and index, not the ingestion logic. Move the Omi/Opus decode path out of scope —
`omi-decoder` stays Rust.

**Gate:** a recorded capture produces the same VFS bundle and records as the
Rust Spore, and a Soil test can feed a fixture event without a host process.

## P2 — data and orchestration

These are good Soil targets after the first two prove the pattern. None should
be ported before their APIs are exercised by a real Soil consumer.

| Spore | What moves | What stays |
|---|---|---|
| `spotify` | playback/transport commands, state shaping | nothing significant |
| `weather` | fetch, normalize, write `weather.current` | host HTTP policy |
| `vitals` | fusion rules, record schema, snapshots | BLE inputs |
| `pulse` | schedule evaluation, webhook state, summaries | ingress host |
| `worklog` | event normalization, ordering, queries | connectors |
| `fleet` | topology shaping and summaries | Mycelium peer transport |
| `git` | landing-queue state and decisions | git host |
| `files` | VFS traversal, census, view model | VFS host |
| `search` | ranking, actions, result shaping | index host |

The pattern is consistent: the Spore keeps the policy, the host keeps the
capability. Porting these also gives Soil a corpus of real programs instead of
UI demos.

## P3 — UI

Do not port one Lua/GPUI view at a time. Wait for `soil:ui` signals and the
HTML/CSS path in the Soil roadmap, then port views in this order:

1. `launcher` / `home` — the smallest state machines with the most reuse;
2. `marbles-ui` — a real data view (and the surviving Marbles UI);
3. `scribe-ui` — capture browsing and search;
4. `lights-view` — the Hue view over the P0 library.

GPUI stays for the surfaces that need its layout/text fidelity:
`lights-view` until the Soil path can draw it, `vitals-view`, and any view
whose whole value is a GPU ring or chart.

## Delete and fold

These are the changes that reduce surface area without losing a capability.

### Capability fixtures

- `probe-gpui` — a host-inspection probe. Fold its checks into the Soil
  capability tests.
- `media-probe` — a `soil:media` fixture. Move the assertion into the media
  host tests.
- `surface-probe` — a `soil:surface-import` fixture. Move the assertion into
  the surface-import tests.

`health-smoke` is different: it is a deployment canary for the runner and
metrics. Keep it.

### Duplicate views

- `chrome` and `divebar` describe the same system bar. `divebar` is the newer
  composition over the current SDK; keep `divebar`, delete `chrome`.
- `board` and `marbles-ui` are two views over the same records book.
  `marbles-ui` is the newer web path; keep it, delete `board` after checking
  that no launcher entry points at `board`.
- `spore-browser` and `launcher` both browse installed Spores. Keep
  `launcher`; fold any unique catalog behavior into it and retire
  `spore-browser` plus its host once `launcher` covers the flow.
- `gallery` is a dashboard-primitive demo. Move the examples into the Soil UI
  docs/tests when that path lands, then delete the package.

### Private

`jarvis-brief`, `jarvis-inbox`, and `jarvis-telemetry` are personal workflow
spores. They belong in the private `ajmwagar/spores` repo, not the public one,
unless they gain a reusable non-personal interface.

## Shared libraries and shims

Do not delete these until the last consumer moves:

| Path | Reason to keep for now |
|---|---|
| `hue-client` | used by the Rust `hue` CLI and `lights-view`; retire with the Hue port |
| `lights-core` | shared Hue state for `lights-view`; retire with the view port |
| `cli-http` | narrow HTTP seam used by Rust CLI Spores |
| `terminal-core` / `terminal-gpui` | terminal semantics and view |
| `watch-core`, `colmi`, `infinitime`, `pinetime` | device transports and watch faces |
| `metrics-sdk` | guest counters/gauges for `health-smoke` and hosts |
| `chrome-host`, `launcher-host`, `spore-browser-host` | native host boundaries |
| `spore-serve`, `spore-catalog`, `themes` | package/catalog tooling until `tend` owns it |

The retirement rule is one commit: remove the consumer and the library together,
or leave both with a named follow-up. A library with no consumer is the same
kind of dead surface as a probe.

## Sequencing

1. **Hue** — land the Soil library and native REPL (in progress).
2. **Deadwax** — port controller logic; keep the view.
3. **Scribe** — port ingestion; keep the web UI.
4. **Delete the fixtures and duplicates** — one mechanical PR per group, after
   grepping launcher/catalog entries.
5. **Spotify/weather/vitals/pulse** — port data and policy as Soil libraries.
6. **UI** — only after `soil:ui` has a stable signal and HTML/CSS contract.
7. **Retire shims** — in the same PR as their last consumer.

Each step must leave the tree runnable. A port is not done when the new code
runs once; it is done when the old path is gone or explicitly listed as
deliberate WIP.

## Non-goals

- Rewriting `markdown`, `omi-decoder`, `dfm-report`, or the GPUI renderers in
  Soil before Soil has a parser/GPU/storage primitive for them.
- Porting every Lua UI file by hand. The UI path is a separate phase.
- Moving capability policy into Spores. The host keeps origins, credentials,
  byte budgets, and device access.
- Keeping a probe or a duplicate "because it still builds". A package that
  exists only to prove a host is a test, and tests belong with the host.
