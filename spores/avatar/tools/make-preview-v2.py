#!/usr/bin/env python3
"""Render the static HTML review page from the generated v2 manifest."""
from __future__ import annotations
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
OUT = HERE / "artifacts" / "v2"
m = json.loads((OUT / "manifest.json").read_text())
v1 = json.loads((HERE / "artifacts" / "manifest.json").read_text())["candidates"]
cards = []
for c in m["candidates"]:
    gif = c["gif"]
    cards.append(f'''<article class="candidate" id="{c['id']}">
  <div class="pair">
    <a class="tile light" href="{c['png256']['path']}"><img src="{c['png256']['path']}" width="196" height="196" alt="{c['name']} static v2 avatar, light surface"><span class="sizes"><img src="{c['png32']['path']}" width="32" height="32" alt="32px"><img src="{c['png24']['path']}" width="24" height="24" alt="24px"></span></a>
    <div class="tile dark"><img src="{c['png256_dark']['path']}" width="196" height="196" alt="{c['name']} static v2 avatar on a dark surface"><span class="sizes"><img src="{c['png32_dark']['path']}" width="32" height="32" alt="32px"><img src="{c['png24_dark']['path']}" width="24" height="24" alt="24px"></span></div>
  </div>
  <h2>{c['name']}</h2><p>{c['family']} · <code>{c['id']}</code></p>
  <details class="motion"><summary>Motion · GIF 96×96 · {gif['frames']} frames · {gif['bytes']:,} bytes</summary>
    <p>{c['motion']}. Static frame is the fallback; open only if motion is welcome.</p>
    <img src="{gif['path']}" width="96" height="96" alt="{c['name']} deterministic animated avatar">
  </details>
</article>''')
v1cards = "\n".join(
    f'''<a class="v1" href="artifacts/png/{c['seed']}-256.png"><img src="artifacts/svg/{c['seed']}.svg" width="64" height="64" alt="{c['candidate']} v1"><span>{c['candidate']}</span></a>'''
    for c in v1
)
html = f'''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Canonical intelligence avatars v2 · review</title>
<style>
:root{{--ink:#17181c;--paper:#f5f1e7;--muted:#6b6b66;--line:#d8d0c2;--dark:#111318;--cream:#eee8dc}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--paper);color:var(--ink);font:15px/1.45 ui-sans-serif,system-ui,-apple-system,Segoe UI,sans-serif}}
main{{max-width:1400px;margin:0 auto;padding:36px 28px 72px}}h1{{font-size:34px;line-height:1.05;margin:0 0 8px;letter-spacing:-.03em}}.lede{{max-width:880px;color:var(--muted);margin:0 0 8px}}.meta{{font:12px ui-monospace,SFMono-Regular,monospace;color:var(--muted);margin:0 0 26px}}code{{font:12px ui-monospace,SFMono-Regular,monospace;background:#e5ded1;padding:1px 5px;border-radius:4px}}
.hero{{display:block;max-width:100%;height:auto;border:1px solid var(--line);border-radius:14px;margin:18px 0 30px;background:#faf7f0}}
.grid{{display:grid;grid-template-columns:repeat(3,1fr);gap:24px}}@media(max-width:980px){{.grid{{grid-template-columns:repeat(2,1fr)}}}}@media(max-width:620px){{.grid{{grid-template-columns:1fr}}}}
.candidate{{border-top:1px solid var(--line);padding-top:12px}}.pair{{display:grid;grid-template-columns:1fr 1fr;gap:8px}}.tile{{padding:10px;border-radius:12px;min-height:248px;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:10px;color:inherit;text-decoration:none}}.light{{background:#faf7f0;border:1px solid #e1d9cc}}.dark{{background:var(--dark);border:1px solid #282b32}}.tile>img{{width:196px;height:196px;display:block}}.sizes{{display:flex;align-items:center;gap:10px;min-height:32px}}.candidate h2{{font-size:14px;margin:10px 0 2px}}.candidate p{{font-size:11px;color:var(--muted);margin:0}}
.section{{margin-top:44px;padding-top:22px;border-top:1px solid var(--line)}}.section h2{{font-size:20px;margin:0 0 8px}}.note{{border-left:3px solid #b8875b;padding:8px 0 8px 14px;color:var(--muted);margin:22px 0;max-width:880px}}.v1grid{{display:grid;grid-template-columns:repeat(8,1fr);gap:10px}}@media(max-width:980px){{.v1grid{{grid-template-columns:repeat(4,1fr)}}}}.v1{{display:flex;flex-direction:column;gap:5px;align-items:center;background:#faf7f0;border:1px solid #e1d9cc;border-radius:10px;padding:8px;color:var(--ink);text-decoration:none;font-size:10px;text-align:center}}.v1 img{{width:64px;height:64px;display:block}}.motion{{margin-top:8px;font-size:11px;color:var(--muted)}}.motion summary{{cursor:pointer;color:var(--ink)}}.motion p{{margin:6px 0}}.motion img{{display:block;margin:6px 0;border-radius:8px;image-rendering:auto}}@media(prefers-reduced-motion:reduce){{.motion img{{display:none}}.motion p::after{{content:" Motion image hidden by reduced-motion preference.";font-weight:600}}}}
</style></head><body><main>
<h1>Canonical intelligence avatars · v2 review</h1>
<p class="lede">Second pass for the founder review. v1 is preserved unchanged below. v2 keeps the same deterministic identity contract and adds four silhouette families with three genuinely different bodies each: orb, visor, organic, sentinel. All geometry and animation frames are pure Soil; rasterization uses the existing <code>rsvg-convert</code>, GIF encoding uses the installed Pillow.</p>
<p class="meta">12 candidates · v2 · 6 frames each · static 256/32/24 PNG · GIF 96×96 · light/dark · source: spores/avatar/source/avatar-v2.soil</p>
<a href="artifacts/v2/comparison-v1-v2.png"><img class="hero" src="artifacts/v2/comparison-v1-v2.png" alt="Side-by-side contact sheet: v1 baseline (16) and v2 diverse candidates (12)"></a>
<div class="note"><strong>Static first.</strong> Mattermost profile images and GitHub avatars are treated as static surfaces here; animated GIF support is not claimed. Each card ships a PNG fallback. Motion is opt-in, and <code>prefers-reduced-motion</code> hides the GIF. No profile was changed.</div>
<div class="grid">{''.join(cards)}</div>
<section class="section"><h2>v1 baseline — preserved</h2><p class="lede">These 16 marks are unchanged from v1 and are shown only for comparison. Their SVGs, PNGs, preview, manifest, and contract remain intact.</p><div class="v1grid">{v1cards}</div></section>
<section class="section"><h2>v2 families</h2><p class="lede"><strong>Orb</strong> (full slit, crescent, twin) · <strong>Visor</strong> (shard, split jaw, oval) · <strong>Organic</strong> (leaf, seed, tendril) · <strong>Sentinel</strong> (monolith, gate, antenna). Color is restrained: ink, bone, and one muted accent. Differences are silhouette-level, not rotations or palette swaps.</p>
<p class="lede">Motion proof: <a href="artifacts/v2/filmstrip-motion.png">six-frame filmstrips for one candidate per family</a> show the deterministic blink, breathe, morph, and orbit sequences without loading the GIFs.</p>
<p class="lede">Small-size evidence: every card shows the actual 32px and 24px PNGs at native size; <a href="artifacts/v2/legibility-24-32.png">this native-size proof sheet</a> also includes a 3× nearest-neighbour inset for inspection. GIFs are 4.5–7.2 KB and six deterministic frames at 140 ms. The contact sheets are at <code>artifacts/v2/contact-sheet-light-12.png</code>, <code>artifacts/v2/contact-sheet-dark-12.png</code>, and <code>artifacts/v2/comparison-v1-v2.png</code>.</p></section>
</main></body></html>'''
(HERE / "preview-v2.html").write_text(html)
print(f"wrote {HERE/'preview-v2.html'}")
