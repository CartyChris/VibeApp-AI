#!/usr/bin/env python3
"""Generate the VibeApp AI source icon (1024x1024 RGBA PNG) with no third-party deps."""
import math
import struct
import zlib
import sys

S = 1024


def lerp(a, b, t):
    return a + (b - a) * t


def mix(c1, c2, t):
    return tuple(lerp(c1[i], c2[i], t) for i in range(3))


# Squircle (superellipse) mask, matching Apple's icon silhouette.
def squircle_alpha(x, y, n=5.0, r=0.5):
    cx, cy = (S - 1) / 2.0, (S - 1) / 2.0
    u = abs(x - cx) / (S * r)
    v = abs(y - cy) / (S * r)
    d = (u ** n + v ** n) ** (1.0 / n)
    # antialias band
    return max(0.0, min(1.0, (1.0 - d) * 90.0))


STOPS = [
    (0.00, (94, 92, 255)),    # indigo
    (0.38, (150, 84, 255)),   # violet
    (0.68, (232, 78, 190)),   # magenta
    (1.00, (255, 138, 96)),   # warm coral
]


def gradient(t):
    t = max(0.0, min(1.0, t))
    for i in range(len(STOPS) - 1):
        a, ca = STOPS[i]
        b, cb = STOPS[i + 1]
        if t <= b:
            return mix(ca, cb, (t - a) / (b - a))
    return STOPS[-1][1]


rows = []
cx = cy = (S - 1) / 2.0

for y in range(S):
    row = bytearray()
    for x in range(S):
        a = squircle_alpha(x, y)
        if a <= 0.0:
            row += b"\x00\x00\x00\x00"
            continue

        # diagonal gradient base
        t = (x / S) * 0.55 + (y / S) * 0.45
        r, g, b = gradient(t)

        # soft radial highlight, upper-left
        hx, hy = S * 0.30, S * 0.24
        hd = math.hypot(x - hx, y - hy) / (S * 0.72)
        hl = max(0.0, 1.0 - hd) ** 2.2 * 0.42
        r = lerp(r, 255, hl)
        g = lerp(g, 255, hl)
        b = lerp(b, 255, hl)

        # bottom vignette for depth
        vg = max(0.0, (y / S - 0.55) / 0.45) ** 1.6 * 0.22
        r, g, b = r * (1 - vg), g * (1 - vg), b * (1 - vg)

        # --- glyph: three stacked "vibe" waveform bars forming an A/spark ---
        nx = (x - cx) / (S * 0.5)
        ny = (y - cy) / (S * 0.5)

        ink = 0.0
        # sine waveform sweeping across the face
        for k, (amp, freq, phase, thick, weight) in enumerate([
            (0.30, 2.05, 0.00, 0.052, 1.00),
            (0.19, 3.10, 1.05, 0.034, 0.55),
            (0.11, 4.30, 2.35, 0.024, 0.32),
        ]):
            if abs(nx) > 0.66:
                continue
            wy = amp * math.sin(nx * math.pi * freq + phase)
            # taper the stroke toward the ends
            taper = max(0.0, 1.0 - (abs(nx) / 0.66) ** 3.0)
            th = thick * (0.35 + 0.65 * taper)
            d = abs(ny - wy)
            v = max(0.0, min(1.0, (th - d) / (th * 0.55 + 1e-6)))
            ink = max(ink, v * weight * taper)

        # spark dot at the crest
        sd = math.hypot(nx - 0.0, ny + 0.30) / 0.075
        ink = max(ink, max(0.0, min(1.0, (1.0 - sd) * 3.0)))

        if ink > 0:
            r = lerp(r, 255, ink * 0.94)
            g = lerp(g, 255, ink * 0.94)
            b = lerp(b, 255, ink * 0.94)

        row += bytes((int(r), int(g), int(b), int(a * 255)))
    rows.append(bytes(row))

raw = b"".join(b"\x00" + r for r in rows)


def chunk(tag, data):
    c = struct.pack(">I", len(data)) + tag + data
    return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)


png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", S, S, 8, 6, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(raw, 9))
    + chunk(b"IEND", b"")
)

out = sys.argv[1] if len(sys.argv) > 1 else "app-icon.png"
with open(out, "wb") as f:
    f.write(png)
print(f"wrote {out} ({len(png)} bytes, {S}x{S})")
