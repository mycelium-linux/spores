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
```

`watch` starts a real-time reading per stream, drains notifications for
`--hold-ms`, then stops what it started so the ring is not left streaming.
`--streams steps` polls the day's activity buckets after the drain. `steps`
polls without streaming anything.

Two rings advertise the same name. Use `--device <serial>` (the DIS serial) to
pick which one; without it the first name containing `colmi` wins.

## Identity

The serial comes from the standard Device Information service. A ring that
exposes none needs `--device <provisioned-id>`; the BLE address is never used
as identity.

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
