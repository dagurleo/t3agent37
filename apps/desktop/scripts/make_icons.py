"""Pixel-art icons for t3agent37: the menu-bar template icon and the app icon.

Writes PNGs with only the standard library (zlib), so no image packages are needed.
  python3 make_icons.py <out-dir>
"""
import math
import struct
import sys
import zlib
from pathlib import Path


def png(path, w, h, rows):
    """rows: list of h lists of w (r, g, b, a) tuples."""
    raw = b"".join(b"\x00" + bytes(c for px in row for c in px) for row in rows)

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
    data += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    Path(path).write_bytes(data)


def scale(grid, k):
    return [[px for px in row for _ in range(k)] for row in grid for _ in range(k)]


def hexc(h, a=255):
    h = h.lstrip("#")
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), a)


CLEAR = (0, 0, 0, 0)
BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5]


def bayer(x, y):
    return (BAYER[(y & 3) * 4 + (x & 3)] + 0.5) / 16


def ramp(colors, t, x, y):
    t = min(max(t, 0.0), 1.0) * (len(colors) - 1)
    i = int(t)
    if i >= len(colors) - 1:
        return colors[-1]
    return colors[i + 1] if t - i > bayer(x, y) else colors[i]


def hash2(a, b, c=0):
    h = ((a * 374761393) ^ (b * 668265263) ^ (c * 2147483647)) & 0xFFFFFFFF
    h = ((h ^ (h >> 13)) * 1274126177) & 0xFFFFFFFF
    h ^= h >> 16
    return h / 4294967296


# ── menu-bar icon: a template image (black + alpha) of a fedora, sunglasses and collar ──
TRAY = [
    "......................",
    "......................",
    "........######........",
    ".......########.......",
    ".......########.......",
    ".......########.......",
    "......................",
    "...################...",
    "..##################..",
    "......................",
    "......###########.....",
    "......####...####.....",
    "......................",
    "......................",
    ".....##.........##....",
    "....####.......####...",
    "...######.....######..",
    "..########...########.",
    "..#########.#########.",
    "..#########.#########.",
    "......................",
    "......................",
]


def tray_grid():
    return [[(0, 0, 0, 255) if ch == "#" else CLEAR for ch in row] for row in TRAY]


# ── app icon: a field agent on a rooftop at dusk, 64×64 grid scaled to 1024 ──
SKY = [hexc(c) for c in ["#2a2235", "#3b2c45", "#5a3b52", "#8a4f57", "#c26f55", "#e9a46a", "#f6d3a4"]]
ROOF = [hexc(c) for c in ["#e2cdb8", "#c5a58a", "#9b7a68", "#6e5560", "#4a3a4f"]]
FAR, NEAR, LIT = hexc("#6e5560"), hexc("#2a2235"), hexc("#ffcf7a")
LEDGE, LEDGE_SHADE = hexc("#f3e7da"), hexc("#4a3a4f")
HAT, BAND, FACE, FACE_SHADE, GLASSES = hexc("#1b1622"), hexc("#c26f55"), hexc("#f6e3cf"), hexc("#d9b99a"), hexc("#0d0b10")
COAT, COAT_SHADE, SHIRT, TIE = hexc("#c9a77c"), hexc("#a8865e"), hexc("#fdfaf6"), hexc("#8a4f57")
BEAM = hexc("#fbeedd")

N = 64
M = 5  # margin around the rounded square
R = 6  # corner radius in grid px
HORIZON = 37


def inside(x, y):
    if x < M or y < M or x >= N - M or y >= N - M:
        return False
    cx = min(max(x, M + R), N - M - 1 - R)
    cy = min(max(y, M + R), N - M - 1 - R)
    return (x - cx) ** 2 + (y - cy) ** 2 <= R * R + 1


def app_grid():
    g = [[CLEAR] * N for _ in range(N)]

    def put(x, y, c):
        if 0 <= x < N and 0 <= y < N and inside(x, y):
            g[y][x] = c

    # sky, with a searchlight beam swinging up from the right
    lamp = (50, 22)
    for y in range(N):
        for x in range(N):
            if y > HORIZON:
                continue
            t = (max(y - M, 0) / (HORIZON - M)) ** 1.6
            dx, dy = x - lamp[0], y - lamp[1]
            if dy < 0:
                a = math.atan2(dy, dx) - (-2.1)
                a = math.atan2(math.sin(a), math.cos(a))
                d = math.hypot(dx, dy)
                if abs(a) < 0.07 + 1.2 / (d + 10):
                    t += 0.5 * (1 - d / 60)
            put(x, y, ramp(SKY, t, x, y))
    # stars high up
    for y in range(M, 16):
        for x in range(M, N - M):
            if hash2(x, y, 3) > 0.975:
                put(x, y, hexc("#fbeedd"))
    # far skyline
    x = M
    i = 0
    while x < N - M:
        w = 4 + int(hash2(i, 10) * 5)
        h = 6 + int(hash2(i, 11) * 9)
        for xx in range(x, x + w):
            for yy in range(HORIZON - h, HORIZON + 1):
                lit = (xx - x) % 2 == 1 and (HORIZON - yy) % 3 == 1 and hash2(xx, yy, 7) > 0.65
                put(xx, yy, hexc("#a98d98") if lit else FAR)
        x += w + (1 if hash2(i, 12) > 0.7 else 0)
        i += 1
    # near skyline with lit windows
    x = M - 2
    i = 0
    while x < N - M:
        w = 4 + int(hash2(i, 20) * 4)
        h = 3 + int(hash2(i, 21) * 6)
        for xx in range(x, x + w):
            for yy in range(HORIZON - h, HORIZON + 1):
                win = (xx - x) % 2 == 1 and (HORIZON - yy) % 2 == 0 and HORIZON - yy > 1 and yy > HORIZON - h + 1
                put(xx, yy, (LIT if hash2(xx, yy, 5) > 0.45 else hexc("#5a3b52")) if win else NEAR)
        x += w + (1 if hash2(i, 22) > 0.7 else 0)
        i += 1
    # the searchlight's building
    for xx in range(46, 55):
        for yy in range(lamp[1] + 2, HORIZON + 1):
            put(xx, yy, NEAR)
    for xx in range(49, 52):
        put(xx, lamp[1] + 1, NEAR)
        put(xx, lamp[1], hexc("#ffe2a8"))
    # roof
    for xx in range(N):
        put(xx, HORIZON + 1, LEDGE)
        put(xx, HORIZON + 2, LEDGE_SHADE)
    for yy in range(HORIZON + 3, N):
        for xx in range(N):
            d = (yy - HORIZON - 3) / (N - M - HORIZON - 3)
            put(xx, yy, ramp(ROOF, 0.35 + d * 0.55 + (hash2(xx, yy, 2) - 0.5) * 0.15, xx, yy))

    # the agent: hat, sunglasses, trench coat with the collar up
    agent = {
        "H": HAT, "h": BAND, "F": FACE, "f": FACE_SHADE, "G": GLASSES, "C": COAT, "c": COAT_SHADE, "T": SHIRT, "t": TIE,
    }
    bust = [
        "..........HHHHHHHHHH..........",
        ".........HHHHHHHHHHHH.........",
        ".........HHHHHHHHHHHH.........",
        ".........HHHHHHHHHHHH.........",
        ".........HHHHHHHHHHHH.........",
        ".........hhhhhhhhhhhh.........",
        ".....HHHHHHHHHHHHHHHHHHHH.....",
        "....HHHHHHHHHHHHHHHHHHHHHH....",
        "..........FFFFFFFFFF..........",
        ".........GGGGGGGGGGGG.........",
        ".........FGGGGFFGGGGF.........",
        ".........FFFFFFFFFFFF.........",
        ".........FFFFFFFFFFFf.........",
        ".........FFFFFFFFFFff.........",
        "..........FFFFFFFFff..........",
        "...........ffffffff...........",
        "........ccCCTTttTTCCcc........",
        "......cCCCCCTTttTTCCCCCc......",
        ".....cCCCCCCCTttTCCCCCCCc.....",
        "....cCCCCCCCCTttTCCCCCCCCc....",
        "...cCCCCCCCCCCttCCCCCCCCCCc...",
        "..cCCCCCCCCCCCttCCCCCCCCCCCc..",
        "..cCCCCCCCCCCCCCCCCCCCCCCCCc..",
        ".cCCCCCCCCCCCCCCCCCCCCCCCCCCc.",
        ".cCCCCCCCCCCCCCCCCCCCCCCCCCCc.",
        ".cCCCCCCCCCCCCCCCCCCCCCCCCCCc.",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
        "cCCCCCCCCCCCCCCCCCCCCCCCCCCCCc",
    ]
    ox, oy = (N - len(bust[0])) // 2, N - M - len(bust)
    for r, row in enumerate(bust):
        for c, ch in enumerate(row):
            if ch != ".":
                put(ox + c, oy + r, agent[ch])
    # a glint on the glasses
    put(ox + 10, oy + 10, BEAM)
    put(ox + 16, oy + 10, BEAM)
    return g


if __name__ == "__main__":
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    t = tray_grid()
    png(out / "tray.png", 22, 22, t)
    png(out / "tray@2x.png", 44, 44, scale(t, 2))
    a = app_grid()
    png(out / "app-icon.png", 1024, 1024, scale(a, 16))
    png(out / "preview-tray.png", 176, 176, scale(t, 8))
    print("written to", out)
