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
```

`watch` starts a real-time reading per stream, drains notifications for
`--hold-ms`, then stops what it started so the ring is not left streaming.
`--streams steps` polls the day's activity buckets after the drain. `steps`
polls without streaming anything.

## Two identical rings

Both advertise the same name. `probe` connects to each candidate in turn and
reports what it answers — handle, DIS serial, model, firmware, battery — and
whether any two report the same serial:

```sh
colmi-ring probe
# {"ok":true,"distinct":true,"rings":[{"handle":2,"name":"COLMI Ring 2",
#   "serial":"sim-ring","battery_percent":74}, ...]}
```

Use `--device <serial>` afterwards to say which ring a session is for; without
it the first name containing `colmi` wins. A duplicate serial is reported in
`duplicates` rather than quietly picking one, because two units claiming one
serial is the case where records would merge two bodies.

The platform address (MAC, or a CoreBluetooth UUID on macOS) also tells the two
apart at scan time, but it is host-local and rotating on privacy-enabled
devices, so Soil keeps it behind the opaque handle and never uses it as
identity. Bonding would only help if the rings paired and used resolvable
private addresses; the QRing flow works unpaired, and nothing here initiates a
bond.

## Identity

The serial comes from the standard Device Information service. A ring that
exposes none needs `--device <provisioned-id>`; the BLE address is never used
as identity.

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
characteristic first — it is the answer that decides whether a `--device` is
needed at all. Every read still reports its own outcome, so "no answer" is
never shown as "no data".

So these units carry **no usable device identity**: the serial characteristic
exists, answers, and is empty. Two rings are told apart only by a host-local
address or by provisioning, which is why `--device` is mandatory here rather
than a fallback.

This is a property of the ring, not of two rings sharing a room: the same
per-read behaviour reproduces with a single ring in range.

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
