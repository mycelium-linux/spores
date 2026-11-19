# omi-ingest

Receives one Omi capture bundle over the host-owned WSS stream and writes it
into Soil.

The sender is `omi-rs` after batch diarization. The bundle is audio and the
canonical `event.json`; this Spore streams the audio through `soil:vfs-mut`,
then materializes the Scribe files (`capture.json`, `transcript.json`,
`summary.md`, `actions.json`) from the event. Audio never enters guest memory
whole.

## Run

```sh
soil spore exec spores/omi-ingest \
  --wss-listen 0.0.0.0:8096 \
  --wss-token "$SOIL_STREAM_TOKEN" \
  --vfs-root ~/.local/share/fungos/vfs
```

Put the TLS terminator in front for a remote sender. `omi-rs` wants:

```sh
OMI_SOIL_WSS=wss://soil.example/stream?token=$SOIL_STREAM_TOKEN
```

## Wire format

All integers big-endian. One WebSocket session carries one bundle.

```text
"OMI1"
repeat:
  u16 path length
  path bytes        e.g. captures/omi/2026-11-19/<capture>/audio.wav
  u64 data length
  data bytes
u16 zero            end of bundle
```

The receiver answers `OK\n` only after every file and the materialized bundle
are committed. A dropped connection leaves the sender's spool entry pending.
