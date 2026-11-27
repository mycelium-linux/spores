#!/usr/bin/env python3
"""Build deterministic v2 avatar artifacts.

Soil owns all geometry and frame generation. This script only invokes soil-lang,
rasterizes SVG with the existing rsvg-convert, and encodes GIFs/contact sheets
with the already-installed Pillow. No network, model, clock, or random input.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent.parent
OUT = HERE / "artifacts" / "v2"
SOIL = Path(os.environ.get("SOIL_LANG", Path.home() / ".local/share/soil/bin/soil-lang"))
PRELUDE = Path(os.environ.get("SOIL_PRELUDE", Path.home() / "src/fungOS/soil/crates/soil-lang/stdlib/prelude.soil"))
RSVG = shutil.which("rsvg-convert")
FRAME_COUNT = 6

# Four visual families, three genuinely different bodies per family. The seed is
# synthetic; the production seed remains SHA-256 of the canonical (issuer, subject).
CANDIDATES = [
    {"id": "orb-brass", "name": "Brass Orb", "family": "orb", "variant": 0, "identity": "v2/orb/brass", "motion": "breathe + orbit + blink"},
    {"id": "orb-crescent", "name": "Crescent Orb", "family": "orb", "variant": 1, "identity": "v2/orb/crescent", "motion": "breathe + orbit"},
    {"id": "orb-twin", "name": "Twin Orb", "family": "orb", "variant": 2, "identity": "v2/orb/twin", "motion": "blink + orbit"},
    {"id": "visor-shard", "name": "Shard Visor", "family": "visor", "variant": 0, "identity": "v2/visor/shard", "motion": "breathe + blink"},
    {"id": "visor-split", "name": "Split Visor", "family": "visor", "variant": 1, "identity": "v2/visor/split", "motion": "blink + breathe"},
    {"id": "visor-oval", "name": "Oval Visor", "family": "visor", "variant": 2, "identity": "v2/visor/oval", "motion": "blink + breathe"},
    {"id": "spore-leaf", "name": "Leaf Spore", "family": "organic", "variant": 0, "identity": "v2/organic/leaf", "motion": "breathe + blink"},
    {"id": "spore-seed", "name": "Seed Spore", "family": "organic", "variant": 1, "identity": "v2/organic/seed", "motion": "breathe + blink"},
    {"id": "spore-tendril", "name": "Tendril Spore", "family": "organic", "variant": 2, "identity": "v2/organic/tendril", "motion": "morph + blink"},
    {"id": "sentinel-monolith", "name": "Monolith Sentinel", "family": "sentinel", "variant": 0, "identity": "v2/sentinel/monolith", "motion": "pulse + blink"},
    {"id": "sentinel-gate", "name": "Gate Sentinel", "family": "sentinel", "variant": 1, "identity": "v2/sentinel/gate", "motion": "blink + pulse"},
    {"id": "sentinel-antenna", "name": "Antenna Sentinel", "family": "sentinel", "variant": 2, "identity": "v2/sentinel/antenna", "motion": "pulse + blink"},
]


def run_soil() -> list[str]:
    program = []
    for c in CANDIDATES:
        for frame in range(FRAME_COUNT):
            program.append(f'(println (av2/render "{c["identity"]}" "{c["family"]}" {c["variant"]} {frame}))')
    with tempfile.NamedTemporaryFile("w", suffix=".soil", delete=False) as f:
        f.write("\n".join(program) + "\n")
        program_path = Path(f.name)
    try:
        p = subprocess.run(
            [str(SOIL), "--prelude", str(PRELUDE), str(HERE / "source/avatar.soil"), str(HERE / "source/avatar-v2.soil"), str(program_path)],
            check=True, text=True, capture_output=True,
        )
    finally:
        program_path.unlink(missing_ok=True)
    lines = [line for line in p.stdout.splitlines() if line.strip()]
    expected = len(CANDIDATES) * FRAME_COUNT
    if len(lines) != expected:
        raise SystemExit(f"expected {expected} frames from Soil, got {len(lines)}")
    return lines


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def info(path: Path) -> dict:
    return {"path": str(path.relative_to(HERE)), "bytes": path.stat().st_size, "sha256": sha256(path)}


PALETTES = {
    "orb": ("#0C0F12", "#F1EDE2"),
    "visor": ("#12110F", "#F4EFE3"),
    "organic": ("#101613", "#EFE9DC"),
    "sentinel": ("#0E1114", "#F2EDE1"),
}


def bg_for(family: str) -> tuple[int, int, int]:
    return {"orb": (241, 237, 226), "visor": (244, 239, 227), "organic": (239, 233, 220), "sentinel": (242, 237, 225)}[family]


def dark_svg(svg: str, family: str) -> str:
    """Transparent-background, inverted two-tone derivative for a dark surface."""
    body, ground = PALETTES[family]
    svg = svg.replace(f"fill='{ground}'", "fill='none'", 1)
    svg = svg.replace(f"fill='{body}'", "fill='__DARK_BODY__'")
    svg = svg.replace(f"fill='{ground}'", f"fill='{body}'")
    svg = svg.replace("fill='__DARK_BODY__'", f"fill='{ground}'")
    # Strokes participate in the silhouette too (e.g. the organic tendril).
    svg = svg.replace(f"stroke='{body}'", "stroke='__DARK_STROKE__'")
    svg = svg.replace(f"stroke='{ground}'", f"stroke='{body}'")
    svg = svg.replace("stroke='__DARK_STROKE__'", f"stroke='{ground}'")
    return svg


def render(svg: Path, size: int, png: Path, family: str, dark: bool = False) -> None:
    subprocess.run([RSVG, "-w", str(size), "-h", str(size), str(svg), "-o", str(png)], check=True)
    im = Image.open(png).convert("RGBA")
    bg_color = (17, 19, 24) if dark else bg_for(family)
    bg = Image.new("RGBA", im.size, bg_color + (255,))
    bg.alpha_composite(im)
    bg.convert("RGB").save(png, "PNG", optimize=True)


def make_gif(frames: list[Path], gif: Path) -> None:
    ims = [Image.open(p).convert("RGB") for p in frames]
    pal = [im.quantize(colors=32, method=Image.Quantize.MEDIANCUT) for im in ims]
    pal[0].save(gif, save_all=True, append_images=pal[1:], duration=140, loop=0, optimize=True, disposal=2)


def font():
    try:
        return ImageFont.truetype("/System/Library/Fonts/Supplemental/Arial.ttf", 13)
    except OSError:
        return ImageFont.load_default()


def make_sheet(items: list[dict], path: Path, dark: bool, cols: int, cell: int = 290) -> None:
    rows = (len(items) + cols - 1) // cols
    w, h = cols * cell, rows * (cell + 38)
    bg = (17, 19, 24) if dark else (250, 247, 240)
    fg = (238, 232, 220) if dark else (25, 26, 29)
    muted = (167, 163, 154) if dark else (107, 107, 102)
    im = Image.new("RGB", (w, h), bg)
    d = ImageDraw.Draw(im)
    f = font()
    for i, item in enumerate(items):
        col, row = i % cols, i // cols
        x, y = col * cell, row * (cell + 38)
        png = HERE / (item["png256_dark"]["path"] if dark else item["png256"]["path"])
        art = Image.open(png).convert("RGB")
        art.thumbnail((cell - 30, cell - 30), Image.Resampling.LANCZOS)
        ax = x + (cell - art.width) // 2
        ay = y + 14 + (cell - 30 - art.height) // 2
        im.paste(art, (ax, ay))
        d.text((x + 16, y + cell + 2), item["name"], font=f, fill=fg)
        d.text((x + 16, y + cell + 20), f'{item["family"]} · {item["id"]}', font=f, fill=muted)
    im.save(path, optimize=True)


def comparison(items: list[dict], path: Path) -> None:
    cols = 7
    cell = 220
    rows = 4
    im = Image.new("RGB", (cols * cell, rows * (cell + 20)), (250, 247, 240))
    d = ImageDraw.Draw(im)
    f = font()
    for i, item in enumerate(items):
        col, row = i % cols, i // cols
        x, y = col * cell, row * (cell + 20)
        png = HERE / item["png128"]["path"]
        art = Image.open(png).convert("RGB")
        art.thumbnail((cell - 24, cell - 24), Image.Resampling.LANCZOS)
        im.paste(art, (x + (cell - art.width) // 2, y + 8))
        d.text((x + 8, y + cell - 4), item["id"], font=f, fill=(70, 70, 66))
    im.save(path, optimize=True)


def main() -> None:
    if not SOIL.is_file() or not PRELUDE.is_file() or not RSVG:
        raise SystemExit("missing soil-lang, prelude, or rsvg-convert")
    for d in ["svg", "png", "gif"]:
        (OUT / d).mkdir(parents=True, exist_ok=True)
    rendered = run_soil()
    manifest = {
        "schema": "mycelium.avatar/v2",
        "style_version": "v2",
        "frame_count": FRAME_COUNT,
        "gif": {"size": 96, "duration_ms": 140, "loop": 0, "colors": 32, "fallback": "png/avatar-256.png"},
        "canonical": {
            "vfs_root": "avatars/<seed_hash>/<style_version>/",
            "record_schema": "mycelium.avatar/identity@1",
            "seed": "production: SHA-256 of canonical (issuer, subject); synthetic labels below",
            "no_private_identity_id_in_filename": True,
        },
        "platform_animation": "animated GIF verified locally; Mattermost/GitHub profile surfaces use the static PNG fallback unless separately verified.",
        "candidates": [],
    }
    for i, c in enumerate(CANDIDATES):
        cdir = OUT
        svgs = []
        for frame in range(FRAME_COUNT):
            svg = cdir / "svg" / f'{c["id"]}-f{frame}.svg'
            svg.write_text(rendered[i * FRAME_COUNT + frame] + "\n")
            svgs.append(svg)
        for size, suffix in [(256, "256"), (32, "32"), (24, "24")]:
            render(svgs[0], size, cdir / "png" / f'{c["id"]}-{suffix}.png', c["family"])
        dark = cdir / "svg" / f'{c["id"]}-dark.svg'
        dark.write_text(dark_svg(svgs[0].read_text(), c["family"]))
        for size, suffix in [(256, "256"), (32, "32"), (24, "24")]:
            render(dark, size, cdir / "png" / f'{c["id"]}-{suffix}-dark.png', c["family"], dark=True)
        gif_frames = []
        for frame, svg in enumerate(svgs):
            p = cdir / "gif" / f'{c["id"]}-f{frame}.png'
            render(svg, 96, p, c["family"])
            gif_frames.append(p)
        gif = cdir / "gif" / f'{c["id"]}.gif'
        make_gif(gif_frames, gif)
        # Verify the actual encoded GIF, not the intended animation.
        with Image.open(gif) as check:
            assert check.is_animated and check.n_frames == FRAME_COUNT, (gif, check.n_frames)
            assert check.size == (96, 96), (gif, check.size)
        seed_hash = "sha256:" + hashlib.sha256(c["identity"].encode()).hexdigest()
        entry = {
            **c,
            "style_version": "v2",
            "seed_hash": seed_hash,
            "proposed_vfs_path": f'avatars/{seed_hash.split(":", 1)[1]}/v2/avatar.svg',
            "svg_static": info(cdir / "svg" / f'{c["id"]}-f0.svg'),
            "svg_frames": [info(p) for p in svgs],
            "png256": info(cdir / "png" / f'{c["id"]}-256.png'),
            "png128": info(cdir / "png" / f'{c["id"]}-128.png') if (cdir / "png" / f'{c["id"]}-128.png').exists() else {"path": str((cdir / "png" / f'{c["id"]}-256.png').relative_to(HERE)), "bytes": (cdir / "png" / f'{c["id"]}-256.png').stat().st_size},
            "png32": info(cdir / "png" / f'{c["id"]}-32.png'),
            "png24": info(cdir / "png" / f'{c["id"]}-24.png'),
            "svg_dark": info(dark),
            "png256_dark": info(cdir / "png" / f'{c["id"]}-256-dark.png'),
            "png32_dark": info(cdir / "png" / f'{c["id"]}-32-dark.png'),
            "png24_dark": info(cdir / "png" / f'{c["id"]}-24-dark.png'),
            "gif": {**info(gif), "frames": FRAME_COUNT, "width": 96, "height": 96},
        }
        manifest["candidates"].append(entry)
    # contact sheets need png128; generate from 256 once for both sheets.
    for c in manifest["candidates"]:
        src = HERE / c["png256"]["path"]
        dst = OUT / "png" / f'{c["id"]}-128.png'
        im = Image.open(src).convert("RGB")
        im.resize((128, 128), Image.Resampling.LANCZOS).save(dst, "PNG", optimize=True)
        c["png128"] = info(dst)
    light = manifest["candidates"]
    make_sheet(light, OUT / "contact-sheet-light-12.png", dark=False, cols=4)
    make_sheet(light, OUT / "contact-sheet-dark-12.png", dark=True, cols=4)
    # side-by-side v1/v2 contact sheet: v1 first, then v2, with a divider row.
    v1 = json.loads((HERE / "artifacts/manifest.json").read_text())["candidates"]
    comp = []
    for c in v1:
        comp.append({"id": f'v1 · {c["seed"]}', "png128": {"path": str(Path("artifacts") / "png" / f'{c["seed"]}-128.png')}})
    # v1 has no 128 derivatives; make them in a scratch dir.
    for c in comp:
        src = HERE / "artifacts" / "png" / (c["id"].split(" · ")[1] + "-256.png")
        dst = OUT / "png" / f'v1-{c["id"].split(" · ")[1]}-128.png'
        Image.open(src).convert("RGB").resize((128, 128), Image.Resampling.LANCZOS).save(dst, "PNG", optimize=True)
        c["png128"] = {"path": str(dst.relative_to(HERE))}
    comp += [{"id": f'v2 · {c["id"]}', "png128": c["png128"]} for c in light]
    comparison(comp, OUT / "comparison-v1-v2.png")
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"generated {len(CANDIDATES)} candidates × {FRAME_COUNT} frames; {len(list((OUT/'gif').glob('*.gif')))} GIFs")


if __name__ == "__main__":
    main()
