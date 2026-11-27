#!/usr/bin/env python3
"""Build a native 32/24px legibility contact sheet for v2 review."""
from pathlib import Path
import json
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent.parent
OUT = HERE / "artifacts" / "v2"
m = json.loads((OUT / "manifest.json").read_text())
cols, cell_w, cell_h = 4, 300, 132
rows = (len(m["candidates"]) + cols - 1) // cols
im = Image.new("RGB", (cols * cell_w, rows * cell_h), (250, 247, 240))
d = ImageDraw.Draw(im)
try:
    f = ImageFont.truetype("/System/Library/Fonts/Supplemental/Arial.ttf", 12)
except OSError:
    f = ImageFont.load_default()
for i, c in enumerate(m["candidates"]):
    x, y = (i % cols) * cell_w, (i // cols) * cell_h
    d.text((x + 12, y + 8), c["name"], font=f, fill=(25, 26, 29))
    # Actual 32px and 24px pixels, top-aligned, plus a nearest-neighbour 3x inset.
    p32 = Image.open(HERE / c["png32"]["path"]).convert("RGB")
    p24 = Image.open(HERE / c["png24"]["path"]).convert("RGB")
    im.paste(p32, (x + 12, y + 28))
    im.paste(p24, (x + 54, y + 32))
    im.paste(p32.resize((96, 96), Image.Resampling.NEAREST), (x + 100, y + 22))
    d.text((x + 12, y + 66), "32", font=f, fill=(107, 107, 102))
    d.text((x + 54, y + 66), "24", font=f, fill=(107, 107, 102))
    d.text((x + 100, y + 120), "3× nearest", font=f, fill=(107, 107, 102))
im.save(OUT / "legibility-24-32.png", optimize=True)
print(OUT / "legibility-24-32.png")
