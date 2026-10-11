"""Generates the AI-Hands icon (an original geometric mark: an abstract open hand on a rounded square).

One geometry table drives both outputs, so the committed SVG and the raster files always match:
    python installers/assets/make-icon.py
writes hands-icon.svg, hands-icon.png (256 px) and hands-icon.ico (16-256 px) next to this script.
Needs Pillow.
"""
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
GRID = 256
BG = "#1F3A5F"
FG = "#F2B544"
STROKE = 28
BACKGROUND = (8, 8, 248, 248, 48)  # x0, y0, x1, y1, corner radius
PALM = (70, 128, 206, 206, 34)
FINGERS = [((84, 78), (84, 150)), ((120, 58), (120, 150)), ((156, 64), (156, 150)), ((192, 86), (192, 150))]
THUMB = ((92, 182), (50, 138))


def svg() -> str:
    x0, y0, x1, y1, r = BACKGROUND
    px0, py0, px1, py1, pr = PALM
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {GRID} {GRID}" width="{GRID}" height="{GRID}">',
        f'  <rect x="{x0}" y="{y0}" width="{x1 - x0}" height="{y1 - y0}" rx="{r}" fill="{BG}"/>',
        f'  <g stroke="{FG}" stroke-width="{STROKE}" stroke-linecap="round" fill="none">',
    ]
    for (ax, ay), (bx, by) in FINGERS + [THUMB]:
        parts.append(f'    <line x1="{ax}" y1="{ay}" x2="{bx}" y2="{by}"/>')
    parts += [
        "  </g>",
        f'  <rect x="{px0}" y="{py0}" width="{px1 - px0}" height="{py1 - py0}" rx="{pr}" fill="{FG}"/>',
        "</svg>",
    ]
    return "\n".join(parts) + "\n"


def raster(size: int) -> Image.Image:
    scale = 1024 / GRID  # draw supersampled, then downscale for smooth edges
    big = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    d = ImageDraw.Draw(big)
    s = lambda v: round(v * scale)
    x0, y0, x1, y1, r = BACKGROUND
    d.rounded_rectangle((s(x0), s(y0), s(x1), s(y1)), radius=s(r), fill=BG)
    w = s(STROKE)
    for (ax, ay), (bx, by) in FINGERS + [THUMB]:
        d.line((s(ax), s(ay), s(bx), s(by)), fill=FG, width=w)
        for cx, cy in ((ax, ay), (bx, by)):
            d.ellipse((s(cx) - w // 2, s(cy) - w // 2, s(cx) + w // 2, s(cy) + w // 2), fill=FG)
    px0, py0, px1, py1, pr = PALM
    d.rounded_rectangle((s(px0), s(py0), s(px1), s(py1)), radius=s(pr), fill=FG)
    return big.resize((size, size), Image.LANCZOS)


if __name__ == "__main__":
    (HERE / "hands-icon.svg").write_text(svg(), encoding="utf-8", newline="\n")
    raster(256).save(HERE / "hands-icon.png")
    raster(256).save(HERE / "hands-icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    print("wrote hands-icon.svg, hands-icon.png, hands-icon.ico")
