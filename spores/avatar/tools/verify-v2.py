#!/usr/bin/env python3
"""Independently verify v2 static derivatives, GIF animation, and local preview links."""
from pathlib import Path
import json
import re
from PIL import Image, ImageSequence

HERE = Path(__file__).resolve().parent.parent
OUT = HERE / "artifacts" / "v2"
m = json.loads((OUT / "manifest.json").read_text())
assert m["style_version"] == "v2" and m["frame_count"] == 6
assert len(m["candidates"]) == 12
seen = set()
for c in m["candidates"]:
    assert c["id"] not in seen; seen.add(c["id"])
    for key, size in [("png256", (256, 256)), ("png32", (32, 32)), ("png24", (24, 24)), ("png256_dark", (256, 256)), ("png32_dark", (32, 32)), ("png24_dark", (24, 24))]:
        p = HERE / c[key]["path"]
        with Image.open(p) as im:
            assert im.size == size, (p, im.size)
    gif = HERE / c["gif"]["path"]
    with Image.open(gif) as im:
        frames = list(ImageSequence.Iterator(im))
        assert getattr(im, "is_animated", False) and im.n_frames == 6, (gif, im.n_frames)
        assert im.size == (96, 96), (gif, im.size)
        assert len(frames) == 6
    assert c["gif"]["bytes"] < 12_000, (gif, c["gif"]["bytes"])
# Every relative asset link in the generated preview must exist, except the v1
# reference links that point at preserved v1 paths.
html = (HERE / "preview-v2.html").read_text()
refs = re.findall(r'(?:src|href)="([^"]+)"', html)
missing = [r for r in refs if not r.startswith(("http", "#")) and not (HERE / r).exists()]
assert not missing, missing
print(f"OK 12 candidates; 12 animated GIFs (6 frames, 96x96, <12KB); static 256/32/24; {len(refs)} local preview links")
