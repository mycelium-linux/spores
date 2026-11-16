# PineTime Spore

A concrete wearable Spore for the PineTime running InfiniTime. It owns the
InfiniTime wire formats, the wearer identity, and the typed signal envelope;
the radio transport and the single-host lease stay in `soil-ble-host`.

## Why `pinetime`, not `smartwatch`

`pinetime` is the honest first target: the service UUIDs, HR layout, motion
units, notification framing, and music service are InfiniTime-specific. A
generic `smartwatch` capability would either leak those details into a fake
abstraction or become a profile registry with one row. When a second watch is
actually supported, add a `smartwatch` profile boundary and make this Spore one
implementation of it. The Spore id is `dev.fungos.pinetime` for the same reason:
renaming an id rekeys per-app Canvas settings and records readers.

## Commands

```text
pinetime watch   [--wearer W] [--device ID] [--scan-ms N] [--hold-ms N]
pinetime notify  [--category C] [--title T] [--body B] [--wearer W]
pinetime media   [--artist A] [--track T] [--album L] [--playing true|false]
pinetime find    [--wearer W]
pinetime time    --epoch UNIX [--offset-min N]
pinetime nav     --icon NAME [--narrative T] [--distance D] [--progress 0-100]
pinetime weather --from-record [--id ID]
pinetime weather --epoch UNIX --icon NAME --temp-c C --min-c C --max-c C
                 [--location L] [--sunrise-min N] [--sunset-min N]
                 [--forecast "min,max,icon;min,max,icon"]
pinetime bridge  [--wearer W] [--person P] [--device ID] [--hold-ms N] [--poll-ms N]
```

- `watch` reads DIS model/firmware/serial and battery, subscribes to heart rate,
  step count, motion, music events, call events, and battery, then drains for
  `--hold-ms` (default 5000).
- `notify` writes an ANS New Alert in InfiniTime's
  `<category><amount>\x00<title>\x00<body>` format. `--category` takes the
  spec's names (`simple`, `email`, `news`, `call`, `missed-call`, `sms`,
  `voicemail`, `schedule`, `high-priority`, `instant-message`, `all`) or its
  byte.
- `media` writes playback status, artist, track, and album, then drains music
  events for `--hold-ms`.
- `find` writes Immediate Alert level 2, which makes the watch vibrate.
- `time` writes the Current Time Service: local fields, weekday, and the
  manual-update reason. The watch has no timezone or network, so `--epoch` is
  the caller's instant and `--offset-min` is the local offset to display.
- `nav` writes the four Navigation characteristics: a validated `pure-maps`
  icon name, the instruction, the distance to it, and route progress.
- `weather --from-record` reads the newest `weather.current.v1` record (or the
  one named by `--id`) and writes the frames from it. The record carries what
  the watch shows, so no caller-typed temperature or icon is involved; a
  forecast frame is written when the record carries days, and skipped when it
  does not. The record struct is `weather-protocol`'s, the same crate the
  `weather` Spore writes, so the body has one definition.
- `weather --epoch … --icon …` writes the same frames from flags, for a caller
  that already knows the values. Temperatures are decimal Celsius on the command
  line and hundredths in the packet; sunrise and sunset accept the service's
  `-1` (unknown) and `-2` (does not occur). Mixing record fields into
  `--from-record` is refused rather than silently ignored, and the three
  temperatures are required: the frame has no honest default.

Every write command reports what it sent. `--read-back` also reads each
characteristic immediately after the write and records `true`, `"mismatch"`, or
`"unsupported"` (write-only on that firmware) in the snapshot.

### Bridge: JARVIS on the wrist

`bridge` polls Unibus for `jarvis.notice.v1`, `jarvis.decision.v1`,
`jarvis.reply.v1`, and `jarvis.decision.resolve.v1`. A notice or a final reply
addressed to the watch becomes an ANS SimpleAlert; a canonical yes/no decision
becomes an ANS Call notification. A tap publishes
`jarvis.decision.resolve.v1` with `by.surface: "watch"`, and a small
`Approved`/`Declined`/`Muted` receipt follows so the wrist does not look stuck.
The adapter also publishes `jarvis.delivery.v1` on shown/resolved/muted/expired.
Only one decision is active at a time; up to eight wait in a queue. `--person`
filters by owner (default: the wearer), and `--hold-ms 0` runs until stopped.

Decisions with more than two options, or binary choices that are not canonical
yes/no/approve/deny, are SimpleAlerts rather than two misleading buttons. The
third ANS header byte is a display hint (`0` plain, `1` approval); stock
InfiniTime skips it and shows the Call screen, while the fork can render thumbs.
The watch does not consume `jarvis.query.v1`: it has no free-text input. A
question that needs a wrist answer is a decision (or an agent prompt normalized
into one).

The active decision and the content hashes already applied persist in the
`bridge/jarvis.state` record; a guest restart re-shows an unanswered decision.
Mute is a surface decision, not an answer: it clears the watch and leaves the
domain decision open for another surface.

## Spec coverage

| InfiniTime service | Spore |
|---|---|
| Device Information, Battery, Heart Rate | read and subscribe |
| Alert Notification (`0x2A46`) | `notify`, any category |
| Notification Event (`00020001`) | call responses as signals |
| Music (`0000000x`) | `media` plus event subscription |
| Motion (`0003000x`) | steps and motion signals |
| Navigation (`0001000x`) | `nav`, icon-validated |
| Simple Weather (`0005000x`) | `weather`, current + forecast |
| Current Time (`0x2A2B`) | `time` |
| Immediate Alert | `find` |

Deliberately not implemented: **DFU** and **BLE FS**. Both are file transfers —
firmware images and watch-face assets — and this Spore has no filesystem grant.
DFU in particular is a nine-step state machine that can brick a watch; it belongs
in a host tool with real file access and a recovery path, not in a capability
Spore. Gadgetbridge is the Android app that implements this same InfiniTime wire
protocol; the protocol is what is implemented here, not the app.

## Streams

`--streams` narrows what a session subscribes to and publishes. It can only
narrow: the profile decides what the watch can emit, and a stream it does not
offer is not selectable.

```text
pinetime watch --streams hr,steps          # a health sync
pinetime watch --streams media,calls       # a control session, no health data
pinetime watch                             # every stream the profile declares
```

Streams are `hr`, `steps`, `motion`, `media`, `calls`, `battery`, or `all`. An
unknown name is an error that lists the known ones — a typo must not silently
subscribe to everything. `battery` is device state, not a body signal: it
updates the device record and never writes a sample.

## Shape

Only one of these three knows what a PineTime is:

- **`spores/infinitime`** — the profile: service and characteristic UUIDs,
  packet layouts, notification parsers, and the release each service arrived in.
- **`spores/watch-core`** — the seam: streams, the signal envelope, the device
  record body, the identity rule, and the `WatchProfile` trait.
- **`spores/pinetime`** — this Spore: the BLE session, the lease, the CLI, and
  one profile wired in.

A second watch is a new profile crate and a new Spore that reuses `watch-core`,
not a fork of this one. The id stays `dev.fungos.pinetime` because renaming it
would rekey records readers and per-app Canvas settings.

## Identity and records

- `device_id` comes from the DIS serial. If the firmware exposes no serial, the
  Spore refuses to invent one from the platform BLE address; pass a provisioned
  `--device` id instead. A rotating address must never split one watch into many
  devices.
- `wearer` is explicit and defaults to `owner`. A watch on a guest's wrist is
  the guest's data even when the owner's host saw the packet. The Spore never
  infers a wearer from proximity.
- `device.state` collection: one `device.state.v1` record per device with
  model, firmware, battery, wearer, and monotonic observation time.
- `weather.current` (read only, written by the `weather` Spore): the current
  conditions this Spore sends with `weather --from-record`.
- `bridge` collection: one `jarvis.state.v1` record holding the active
  decision, the bounded queue, the content hashes already applied, and the
  wall-clock anchor used to honour `expires_at_ms` without a wall-clock grant.
- `signal` collection: append-only `signal.sample.v1` records for
  `body.hr.v1`, `body.steps.v1`, `body.motion.v1`, `media.event.v1`, and
  `call.response.v1`.
- A heart-rate measurement of zero is stored as `bpm: null` with `quality: 0`,
  never as a zero BPM. Step counts stay per device; the Spore never sums two
  devices. Raw audio is never written to the bus.

## Grants

| Grant | Why |
|---|---|
| `soil:ble` | observe (scan/services/read/subscribe/next-event), connect (take the radio lease), write (notifications, media, find) |
| `soil:records` | read previous state and the weather record, and write device/signal/bridge records |
| `soil:unibus` | observe JARVIS decision/notice/reply frames and publish delivery/resolution frames |
| `soil:time` | monotonic observation timestamps and bridge poll timing |
| `wasi:cli`, `wasi:clocks`, `wasi:io` | Rust std runtime on `wasm32-wasip2`; no file or network I/O |

No socket, process, secret, environment, raw filesystem, or wall-clock grant.

## Build

```sh
cargo test -p pinetime-spore
cargo test -p infinitime
cargo build -p pinetime-spore --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/pinetime_spore.wasm module.wasm
```

The pure bridge mapping and wire protocol tests run on the host. The Spore crate
is the thin BLE/records wrapper. `soil-spore check .` verifies the manifest
against the module's real imports.

## Host requirement

The module imports `soil:ble`, `soil:records`, `soil:time`, and `soil:unibus`.
On desktop, `soil-dev-host` composes exactly those and runs this Spore against
the paced simulator (or `--real` for CoreBluetooth/BlueZ). `soil-ble-host`
remains the single-protocol BLE CLI and links only `soil:ble` + WASI; `bridge`
belongs in the composed host.

```sh
cargo build -p soil-dev-host

target/debug/soil-dev-host \
  --bus-fixture /tmp/decisions.jsonl \
  --ble-device sim-pinetime \
  spores/pinetime/module.wasm \
  bridge --wearer owner --person owner --hold-ms 3000 --poll-ms 100
```

Each fixture line is `{"kind": "jarvis.decision.v1", "body": {...}}`; the dev
host publishes it into its in-memory Unibus before the guest starts.

## Simulator

`soil-ble-sim` exposes a virtual PineTime with the same InfiniTime services,
deterministic ticks, battery drain, packet loss, disconnects, stale reads, and
replayable golden records. Run the host with `SOIL_BLE_SIM=1` to exercise this
Spore without a radio:

```sh
target/debug/soil-dev-host --records-dir .soil-dev/records \
  module.wasm watch --wearer owner --hold-ms 1000
```

## Status

- The protocol mapping, bridge mapping, and wasm component build are covered by
  tests.
- The records host is required for persistence; without it `run` fails loudly.
- `bridge` is verified end to end against `soil-dev-host` and the paced
  simulator: a fixture decision is shown, the simulator's call-response event
  resolves it, and the resolve/delivery frames appear on the bus.
- Continuous overnight HR logging is not implemented in stock InfiniTime; this
  Spore consumes the notifications the firmware actually emits.
- Stock InfiniTime has no remote dismiss. A resolved decision's original ANS
  entry remains in the watch's notification list until dismissed by hand; the
  adapter does not send a replacement.

## Watch faces

Faces are authored on the host in Lua and compiled to a compact WF1 descriptor
for the native renderer. Lua does not run on the watch; the face only describes
widgets and bindings. [`faces/glance.lua`](faces/glance.lua) is the deployable
single-page reference; [`faces/atlas.lua`](faces/atlas.lua) is the three-page
design preview (home, rings, agenda) for the next WF1 contract. See
[`docs/watchface.md`](../../docs/watchface.md).

```sh
lua tools/watchface.lua faces/glance.lua --json
lua tools/watchface.lua faces/glance.lua --bin /tmp/glance.wf
lua tools/watchface_test.lua
```

## Wearers and records

`--wearer NAME` selects that person's records book. The Spore writes the
`<collection>:<wearer>` namespace (`signal:ange`, `device.state:ange`, …) and
the host maps it to `personal/<wearer>/telemetry`; a wearer the host was not
started with (`--wearers ajm,ange`) is denied by name rather than writing into
someone else's book. Two rings, two wearers, two stores.
