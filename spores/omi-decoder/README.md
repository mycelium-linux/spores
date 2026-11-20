# Omi decoder

Consumes `omi.audio.batch.v1` from the Omi pendant Spore, decodes the 3-byte
framed Opus packets, transcribes bounded PCM windows, writes one `audio.wav`
into the capture bundle, and publishes `omi.capture.v1` for Scribe.

The host owns libopus and the STT route:

```text
SOIL_STT_URL    OpenAI-compatible speech-to-text base URL
SOIL_STT_MODEL  default model when the request names none
```

The Spore never sees a codec handle, a socket, a credential, a host path, or a
wall clock.

```bash
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/omi_decoder_spore.wasm module.wasm
```

Run it next to the Omi pendant Spore:

```bash
soil spore run --manifest spores/omi-decoder/spore.toml -- watch
```

A session is finalized after `--idle-ms` (default 20s) without an audio batch.
One Omi session produces one capture card; unrelated live fragments are never
merged.
