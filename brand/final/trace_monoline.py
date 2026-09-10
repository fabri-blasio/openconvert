"""Trace the selected generated Mole 04 artwork into flat SVG paths.

This intentionally traces the raster source instead of redrawing it from memory.
It uses a small fixed palette, follows pixel boundaries, and simplifies them by
less than two pixels at the 512px working size.
"""

from __future__ import annotations

from collections import defaultdict, deque
from pathlib import Path
from typing import Iterable

import numpy as np
from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "animal-concepts" / "mole-04-monoline.png"
OUTPUT = Path(__file__).resolve().parent / "openconvert-mole-monoline-traced.svg"
SIZE = 1024

PALETTE = {
    "ink": np.array([29, 29, 31], dtype=np.int32),
    "brand": np.array([47, 111, 106], dtype=np.int32),
    "paper": np.array([250, 250, 250], dtype=np.int32),
}


def rdp(points: list[tuple[float, float]], epsilon: float) -> list[tuple[float, float]]:
    if len(points) < 3:
        return points
    start = np.array(points[0], dtype=float)
    end = np.array(points[-1], dtype=float)
    line = end - start
    norm = np.linalg.norm(line)
    if norm == 0:
        distances = [np.linalg.norm(np.array(p) - start) for p in points[1:-1]]
    else:
        distances = [
            abs(line[0] * (p[1] - start[1]) - line[1] * (p[0] - start[0])) / norm
            for p in points[1:-1]
        ]
    if not distances:
        return [points[0], points[-1]]
    index = int(np.argmax(distances)) + 1
    maximum = distances[index - 1]
    if maximum > epsilon:
        left = rdp(points[: index + 1], epsilon)
        right = rdp(points[index:], epsilon)
        return left[:-1] + right
    return [points[0], points[-1]]


def connected_components(mask: np.ndarray) -> list[tuple[int, tuple[int, int, int, int]]]:
    height, width = mask.shape
    seen = np.zeros_like(mask, dtype=bool)
    result: list[tuple[int, tuple[int, int, int, int]]] = []
    for y, x in zip(*np.nonzero(mask)):
        if seen[y, x]:
            continue
        queue = deque([(x, y)])
        seen[y, x] = True
        pixels: list[tuple[int, int]] = []
        while queue:
            px, py = queue.popleft()
            pixels.append((px, py))
            for nx, ny in ((px - 1, py), (px + 1, py), (px, py - 1), (px, py + 1)):
                if 0 <= nx < width and 0 <= ny < height and mask[ny, nx] and not seen[ny, nx]:
                    seen[ny, nx] = True
                    queue.append((nx, ny))
        xs = [p[0] for p in pixels]
        ys = [p[1] for p in pixels]
        result.append((len(pixels), (min(xs), min(ys), max(xs), max(ys))))
    return sorted(result, reverse=True)


def boundary_loops(mask: np.ndarray) -> list[list[tuple[int, int]]]:
    height, width = mask.shape
    outgoing: dict[tuple[int, int], list[tuple[int, int]]] = defaultdict(list)
    ys, xs = np.nonzero(mask)
    for y, x in zip(ys.tolist(), xs.tolist()):
        if y == 0 or not mask[y - 1, x]:
            outgoing[(x, y)].append((x + 1, y))
        if x == width - 1 or not mask[y, x + 1]:
            outgoing[(x + 1, y)].append((x + 1, y + 1))
        if y == height - 1 or not mask[y + 1, x]:
            outgoing[(x + 1, y + 1)].append((x, y + 1))
        if x == 0 or not mask[y, x - 1]:
            outgoing[(x, y + 1)].append((x, y))

    loops: list[list[tuple[int, int]]] = []
    while outgoing:
        start = next(iter(outgoing))
        current = start
        loop = [start]
        for _ in range(width * height * 4):
            choices = outgoing.get(current)
            if not choices:
                break
            nxt = choices.pop()
            if not choices:
                del outgoing[current]
            current = nxt
            if current == start:
                break
            loop.append(current)
        if current == start and len(loop) >= 8:
            loops.append(loop)
    return loops


def path_data(mask: np.ndarray, epsilon: float = 0.45, min_area: int = 5) -> str:
    paths: list[str] = []
    for loop in boundary_loops(mask):
        xs = [p[0] for p in loop]
        ys = [p[1] for p in loop]
        if (max(xs) - min(xs)) * (max(ys) - min(ys)) < min_area:
            continue
        closed = loop + [loop[0]]
        simple = rdp(closed, epsilon)
        if simple[-1] != simple[0]:
            simple.append(simple[0])
        commands = [f"M{simple[0][0]} {simple[0][1]}"]
        commands.extend(f"L{x} {y}" for x, y in simple[1:-1])
        commands.append("Z")
        paths.append("".join(commands))
    return "".join(paths)


image = Image.open(SOURCE).convert("RGBA").resize((SIZE, SIZE), Image.Resampling.LANCZOS)
array = np.asarray(image)
rgb = array[:, :, :3].astype(np.int32)
alpha = array[:, :, 3]
names = list(PALETTE)
colors = np.stack([PALETTE[name] for name in names])
distances = ((rgb[:, :, None, :] - colors[None, None, :, :]) ** 2).sum(axis=3)
labels = distances.argmin(axis=2)
visible = alpha >= 72
masks = {name: visible & (labels == index) for index, name in enumerate(names)}

# The source used verdigris on only the upper-right section of the outer ring.
# Make that ring a single coherent ink contour. The smaller verdigris nose and
# paw accents sit lower in the artwork and remain untouched.
yy, xx = np.ogrid[:SIZE, :SIZE]
outer_green = masks["brand"] & (yy < SIZE * 0.57) & (xx > SIZE * 0.52)
masks["ink"] |= outer_green
masks["brand"] &= ~outer_green

# Close the small source gap where the two ring colours used to meet. This is
# the only geometry repair: it turns the cap-like broken arc into one tunnel.
ink_image = Image.fromarray((masks["ink"] * 255).astype(np.uint8), mode="L")
draw = ImageDraw.Draw(ink_image)
draw.polygon(
    [
        (int(SIZE * 0.486), int(SIZE * 0.093)),
        (int(SIZE * 0.552), int(SIZE * 0.104)),
        (int(SIZE * 0.546), int(SIZE * 0.139)),
        (int(SIZE * 0.490), int(SIZE * 0.130)),
    ],
    fill=255,
)
masks["ink"] = np.asarray(ink_image) > 0

for name, mask in masks.items():
    print(name, connected_components(mask)[:12])

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" role="img" aria-labelledby="title desc">
  <title id="title">OpenConvert traced monoline mole icon</title>
  <desc id="desc">The selected monoline mole artwork, traced into flat vector paths.</desc>
  <rect x="24" y="24" width="976" height="976" rx="232" fill="#FAFAFA"/>
  <path d="{path_data(masks['ink'])}" fill="#1D1D1F" fill-rule="evenodd"/>
  <path d="{path_data(masks['brand'])}" fill="#2F6F6A" fill-rule="evenodd"/>
  <path d="{path_data(masks['paper'])}" fill="#FAFAFA" fill-rule="evenodd"/>
</svg>
'''
OUTPUT.write_text(svg, encoding="utf-8")
print(f"wrote {OUTPUT}")
