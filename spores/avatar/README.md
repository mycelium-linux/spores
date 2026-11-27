# Canonical intelligence avatars — prototype

A pure Soil generator for a stable synthetic visual identity. This is a library
prototype, not a service and not a production profile change.

## Contract

```text
identity tuple (issuer, subject)  -> canonical seed (SHA-256)
style version                     -> "v1"
seed + style version              -> deterministic SVG
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

The seed is a hash of the canonical `(issuer, subject)` tuple. The record and
VFS object are the authority; a CDN URL is a projection. The files contain no
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
