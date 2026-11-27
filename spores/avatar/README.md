# Canonical intelligence avatars — prototype

A pure Soil generator for a stable synthetic visual identity. This is a library
prototype, not a service and not a production profile change.

## Contract

```text
identity tuple (issuer, subject)  -> caller-proposed canonical seed (production: SHA-256)
style version                     -> "v1"
seed + style version              -> deterministic FNV-1a style hash -> SVG
SVG                               -> existing rsvg-convert -> PNG 256/32/24
```

The prototype uses **synthetic seeds** (`atlas-01` … `atlas-16`). No private
identity ID, email, name, or real profile is embedded in any artifact. The
generator is in [`source/avatar.soil`](source/avatar.soil); `tools/generate.sh`
binds candidate names to files and rasterizes with the already-installed
`rsvg-convert`.

## Visual families

- **Fold** (01–12): default recommendation. A restrained aperture/fold with one
  seed-derived notch, a quiet muted rule, and a palette selected from six
  low-saturation combinations. It is original geometry, not a copied logo and
  not a face.
- **Orbit** (13, 15): nested rings with one notch; a rounder alternative.
- **Signal** (14, 16): stacked bars and an offset block; a quieter editorial
  alternative.

All candidates are presented at full display size and at 32px/24px on both light
and dark surfaces in [`preview.html`](preview.html). The full 256px raster is
linked from each light tile.

## Artifacts

```text
artifacts/svg/atlas-01.svg … atlas-16.svg
artifacts/png/atlas-01-256.png … atlas-16-256.png
artifacts/png/atlas-01-32.png … atlas-16-32.png
artifacts/png/atlas-01-24.png … atlas-16-24.png
artifacts/contact-sheet-light-4x4.png
artifacts/contact-sheet-dark-4x4.png
artifacts/manifest.json
preview.html
```

`manifest.json` records the seed, style, style version, file bytes and SHA-256
content ID for every derivative. It also names the proposed canonical VFS path
and record schema.

## Canonical VFS / records shape

```text
VFS:    avatars/<seed-sha256>/v1/avatar.svg
        avatars/<seed-sha256>/v1/avatar-256.png
        avatars/<seed-sha256>/v1/avatar-32.png
        avatars/<seed-sha256>/v1/avatar-24.png
Record: mycelium.avatar/identity@1
        { seed_hash, style_version, svg_content_id, png_content_ids,
          mime, bytes, generated_at }
```

The prototype takes synthetic seed labels; the production seed is proposed as
a SHA-256 of the canonical `(issuer, subject)` tuple (never email or name).
The generator then derives its bounded geometry hash from `seed|style|v1`. The
record and VFS object are the authority; a CDN URL is a projection. The files contain no
private identity ID in their names. The authorized sync contract is the
existing `soil:vfs` + `soil:records` grant: read for face surfaces, write only
for the identity owner/adapter. Do not create a second avatar database.

## Run

```sh
SOIL_LANG=$HOME/.local/share/soil/bin/soil-lang \
SOIL_PRELUDE=$HOME/src/fungOS/soil/crates/soil-lang/stdlib/prelude.soil \
  tools/generate.sh
SOIL_LANG=$HOME/.local/share/soil/bin/soil-lang \
SOIL_PRELUDE=$HOME/src/fungOS/soil/crates/soil-lang/stdlib/prelude.soil \
  tests/run.sh
```

The generator is deterministic: repeating it produces the same SVG and PNG
bytes. The seeds and style version are immutable inputs; changing the geometry
requires a new style version, not silently changing `v1`.

## Integration sketch — no production changes

- **Rakazo**: use the 256px PNG as the public avatar data image or Matrix
  `mxc://` content; store `seed_hash` + `style_version` + content IDs. Reuse
  `rakazo-drawbridge/src/avatar.rs` and its `profiles.avatar_hash/avatar_uri`
  cache; update only when the content ID changes.
- **Mattermost bot profile**: set the bot image through the existing Mattermost
  API `PUT /users/{id}/image` with the 256px PNG. Seed from the bot's stable
  server identity, not its display name or a channel message.
- **Cyberspace / MeatSpace**: use the deterministic mark as the fallback when
  `photo_url` is absent; `Person`/`Agent` already expose initials fallbacks.
  Do not change roster records or create accounts.
- **OIDC `picture`**: treat the provider picture as an override; the generated
  mark is the fallback. Seed from `(issuer, subject)`, never email or name.
- **GitHub**: keep a human GitHub avatar as an override; for synthetic agents,
  use the deterministic mark. Do not call GitHub write APIs.

## Tests

`tests/run.sh` checks same-seed determinism, seed separation, style separation,
style version, self-containment (no external image reference), and all three
families. License notes are in [`LICENSE-NOTES.md`](LICENSE-NOTES.md).

## v2 — founder review second pass

v1 is intentionally preserved unchanged: its source, tests, 16 SVGs, 48 PNGs,
contact sheets, manifest, preview, and `v1` contract remain in place. v2 is a
separate style version and artifact tree.

```text
source/avatar-v2.soil       pure Soil geometry + six deterministic frames
artifacts/v2/svg/           12 × 6 SVG frames
artifacts/v2/png/           static 256/32/24 PNGs (+ 128 review thumbnails)
artifacts/v2/gif/           one 96×96, 6-frame GIF per candidate + frame PNGs
artifacts/v2/manifest.json  seed hash, style v2, content IDs, frame/GIF metadata
artifacts/v2/contact-sheet-light-12.png
artifacts/v2/contact-sheet-dark-12.png
artifacts/v2/comparison-v1-v2.png
artifacts/v2/legibility-24-32.png
preview-v2.html             static-first review page with opt-in motion
```

The v2 contract keeps the v1 authority shape but versions the style:

```text
identity tuple (issuer, subject) -> caller-proposed canonical seed
style_version                    -> "v2"
seed + family + variant + frame  -> pure Soil SVG
static frame 0                   -> rsvg-convert -> PNG 256/32/24
frames 0..5                      -> rsvg-convert 96 + Pillow GIF
Record: mycelium.avatar/identity@1
VFS:    avatars/<seed_hash>/v2/avatar.svg
```

The reference is visual grammar only — high-contrast abstract bot faces, expressive eyes, negative space, orb/organic forms — not a copied logo or character. Four families, three distinct bodies each — no rotation or palette-only variants:

- **orb**: full slit orb, crescent, twin orb
- **visor**: shard, split jaw, oval
- **organic**: leaf, seed, tendril
- **sentinel**: monolith, gate, antenna

Only the first frame is the default. Motion is opt-in, and the static PNG is the
fallback. See [`docs/ANIMATION.md`](docs/ANIMATION.md) for the verification and
the explicit non-claim about Mattermost/GitHub animation.

```sh
python3 tools/generate-v2.py
python3 tools/make-legibility-v2.py
python3 tools/make-preview-v2.py
python3 tools/verify-v2.py
tests/run-v2.sh
```
