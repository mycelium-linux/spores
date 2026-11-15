# COLMI ring Spore

A concrete wearable Spore for the COLMI Ring 2 (QRing protocol). It owns the
QRing session, the wearer identity, and the typed signal envelope; the radio
transport and the per-device lease stay in `soil-ble-host`.

## Why a separate Spore, not a `smartwatch` mode

A ring has no screen, no media service, and no clock UI. Its measurements are
request/response over one UART characteristic pair
(`6e40fff0-…` / write `6e400002-…` / notify `6e400003-…`), and packets are a
fixed 16 bytes with an additive checksum. The PineTime speaks InfiniTime's
services and notifications. What they share — `Stream`, `Signal`, the record
envelope, the identity rule — lives in `spores/watch-core`; what differs lives
in `spores/colmi` (wire) and this Spore (session and commands).

## Commands

```text
colmi-ring watch   [--wearer W] [--device ID] [--scan-ms N] [--hold-ms N]
                   [--streams hr,spo2,steps]
colmi-ring battery [--wearer W] [--device ID]
colmi-ring steps   [--wearer W] [--device ID] [--day N]
colmi-ring probe   [--device ID] [--scan-ms N]
colmi-ring claim   <id> [--handle N] [--scan-ms N]
```

`watch` starts a real-time reading per stream, drains notifications for
`--hold-ms`, then stops what it started so the ring is not left streaming.
`--streams steps` polls the day's activity buckets after the drain. `steps`
polls without streaming anything.

## Two identical rings

Both advertise the same name and expose no serial, so nothing in a scan tells
them apart. Claim each one once, with an id you choose:

```sh
colmi-ring claim ring-left      # with the other ring out of range
colmi-ring claim ring-right     # the only unclaimed ring left
```

Soil stores the binding (id ↔ platform address) in its session file, and every
later scan reports `device-id` before any read, so `--device ring-left` selects
the right ring from then on. A session with no `--device` refuses when more than
one ring matches, instead of picking one:

```text
2 rings match by name and none is selected; pass --device <claimed-id>,
or run `colmi-ring claim <id>` once per ring
```

`probe` lists handles if you would rather say which one directly, but handles
are per scan; the physical ceremony above is the reliable way to bind each unit.

## Identity

Order: **the claimed id**, then a serial the device itself reports, then an
error. `--device` only selects a peripheral — it never becomes a record key, and
the BLE address never does either.

**Verified on two COLMI Ring 2 units (2026-11-14, macOS/btleplug):**

| DIS characteristic | present | read |
| --- | --- | --- |
| 2A25 serial | yes | reads as **0 bytes** |
| 2A24 model | no | — |
| 2A27 hardware | yes | `RT12CR2_V1.0` when it is the first read |
| 2A26 firmware | yes | same constraint; not captured yet |

The ring answers about **one GATT round trip per connection** and then stops
responding: a second read times out, and an immediate reconnect times out too
(~15 s). That is why `probe` takes one connection and reads the identity
characteristic first. Every read still reports its own outcome, so "no answer"
is never shown as "no data".

## Firmware update channel (probe only)

Beside the UART pair the rings expose a second service:

| UUID | role |
| --- | --- |
| `de5bf728-d711-4e47-af26-65e3012a5dc7` | OTA service |
| `de5bf72a-d711-4e47-af26-65e3012a5dc7` | write, not notifiable |
| `de5bf729-d711-4e47-af26-65e3012a5dc7` | notify, not writable |
| `0000fee7-…` | vendor service, purpose unknown |

Verified on both Ring 2 units; `probe` reports the service and each
characteristic's properties. Nothing in this Spore writes firmware — the
service is named so the next step can build on a measured fact instead of
a guess.

The channel is the BlueX RF03 bootloader, documented with dumps, official
images, and a working flasher at
<https://github.com/atc1441/ATC_RF03_Ring>. Its framing is
`BC | type | len u16 | crc16 u16 | payload`, CRC-16/MODBUS, staged: image
length and checksum, then acked chunks. The vendor app (QRing) contains
the same five UUIDs and its OTA response carries `hardwareVersion`,
`version`, and `downloadUrl` — so the image is chosen **server-side** from
the ring's 2A27 value (`RT12CR2_V1.0`), not from a table inside the app.

This is a property of the ring, not of two rings sharing a room: the same
per-read behaviour reproduces with a single ring in range. The simulator models
a ring with an empty 2A25, so the claim path is what gets exercised in tests.

The vendor UART service is confirmed present on the hardware:
`6e40fff0-…` with write `6e400002-…` and notify `6e400003-…`.

## Local development

```sh
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/colmi_ring_spore.wasm module.wasm
```

Run it against the soil simulator (which models a QRing ring, including the
command/response protocol):

```sh
SOIL_HOST_ID=dev soil-dev-host module.wasm watch --wearer owner --hold-ms 1000
```
