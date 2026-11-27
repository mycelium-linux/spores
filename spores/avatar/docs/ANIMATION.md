# v2 avatar animation notes

The v2 motion assets are deterministic frame sequences, not video or model output.
`source/avatar-v2.soil` returns SVG for `(identity, family, variant, frame)`; the
frame index is the only time input. `tools/generate-v2.py` rasterizes the six
frames at 96×96 and encodes a looping GIF with the already-installed Pillow.
There is no random seed, clock, network call, or image model.

## Verified locally

`tools/verify-v2.py` reopens every encoded GIF and asserts:

- 12 candidates, 6 frames each
- `is_animated == True`, `n_frames == 6`
- 96×96 pixels, 32-colour palette, 140 ms/frame, loop forever
- individual GIFs are under 12 KB (actual range: about 4.5–7.2 KB)
- static PNGs are 256×256, 32×32 and 24×24
- every relative link in `preview-v2.html` resolves

The static frame is frame 0 and is the default image in the preview. Animation is
inside an opt-in `<details>` element; no GIF is loaded until a reviewer opens it.
`@media (prefers-reduced-motion: reduce)` hides the motion image.

## Platform support: no unsupported claim

Browser `<img>` animation of GIF is standard and was exercised by decoding the
actual files above. **Mattermost bot profile images and GitHub avatars are not
claimed to preserve animation.** Both are profile-image surfaces that may crop,
re-encode, proxy, or serve a static derivative. The delivery format for those
surfaces is `png256` (`*-256.png`), not the GIF. Do not set a profile from this
prototype; it is a review artifact only.

If a future adapter wants motion, it must test that adapter's actual served bytes
and content type first, and still ship the static PNG fallback.
